//! Sticks of dynamite, dropped with the Dynamite tool. A stick falls through air and water, comes
//! to rest on whatever holds it (ground, powder, a block, another stick) and its fuse burns down
//! while the simulation runs. A stick pushed into matter stays planted there, like a charge in a
//! blast hole: its blast starts from inside.
//!
//! A stick lies over the matter, it never takes its place: only its blast changes that matter. When
//! the fuse is out it goes off (see `world/blast.rs`), and sticks that touch, or lie one over the
//! other, go off as one charge that adds up all of theirs. The shock of a blast sets off the sticks around it
//! a few steps later, before their own fuse (sympathetic detonation): a heap of sticks goes off
//! almost together, much faster than fuse after fuse.

use crate::render::View;
use crate::world::{Kind, World, H, W};
use crate::gfx::*;

/// Size of a stick lying down, cells (5 cm each: a fat stick, so that it can be seen).
pub const LEN: i32 = 10;
pub const THICK: i32 = 3;
/// Steps the fuse burns (3 s at x1).
pub const FUSE: u32 = 180;
/// Pull, cells per step², and the fastest fall in air and in a liquid, cells per step.
const GRAVITY: f32 = 0.12;
const MAX_FALL: f32 = 4.0;
const MAX_SINK: f32 = 0.5;
/// How far the shock of one stick sets off the others, cells (more for a bigger charge).
const SYMPATHY: f32 = 30.0;
/// No more sticks than this at once.
pub const MAX_STICKS: usize = 80;

#[derive(Clone, Debug)]
pub struct Stick {
    /// Top left cell of the stick.
    pub x: i32,
    pub y: i32,
    /// Fall speed, cells per step, and what is left of a step's fall.
    pub vy: f32,
    pub carry: f32,
    /// Steps of fuse left.
    pub fuse: u32,
    /// Pushed into matter: it stays where it is while the matter around it holds.
    pub planted: bool,
}

impl Stick {
    pub fn centre(&self) -> Vec2 {
        vec2(self.x as f32 + LEN as f32 / 2.0, self.y as f32 + THICK as f32 / 2.0)
    }

    fn touches(&self, o: &Stick) -> bool {
        self.x <= o.x + LEN && o.x <= self.x + LEN && self.y <= o.y + THICK && o.y <= self.y + THICK
    }
}

/// Does the cell hold a stick up? Solids, powders and blocks do; air and liquids do not.
fn holds(world: &World, x: i32, y: i32) -> bool {
    if !(0..W() as i32).contains(&x) || !(0..H() as i32).contains(&y) {
        return false;
    }
    matches!(world.raw(x, y).kind(), Kind::Static | Kind::Powder) || world.block_at_cell(x, y).is_some()
}

#[derive(Default, Clone)]
pub struct Dynamite {
    pub sticks: Vec<Stick>,
}

impl Dynamite {
    /// A stick dropped with its middle at `at`. Inside matter it is planted there.
    pub fn drop(&mut self, world: &World, at: Vec2) -> bool {
        if self.sticks.len() >= MAX_STICKS {
            return false;
        }
        let (x, y) = ((at.x - LEN as f32 / 2.0).round() as i32, (at.y - THICK as f32 / 2.0).round() as i32);
        let x = x.clamp(0, W() as i32 - LEN);
        let planted = holds(world, at.x as i32, at.y as i32);
        self.sticks.push(Stick { x, y, vy: 0.0, carry: 0.0, fuse: FUSE, planted });
        true
    }

    pub fn clear(&mut self) {
        self.sticks.clear();
    }

    /// Is stick `i` held up from below (by matter or by another stick)?
    fn supported(&self, world: &World, i: usize) -> bool {
        let s = &self.sticks[i];
        let below = s.y + THICK;
        if (s.x..s.x + LEN).any(|x| holds(world, x, below)) {
            return true;
        }
        self.sticks.iter().enumerate().any(|(j, o)| j != i && o.y == below && o.x < s.x + LEN && s.x < o.x + LEN)
    }

    /// Sticks fall, one cell at a time so they never pass through anything.
    fn fall(&mut self, world: &World) {
        for i in 0..self.sticks.len() {
            let s = &self.sticks[i];
            let c = s.centre();
            if s.planted {
                // Planted while the matter it was pushed into is there.
                if holds(world, c.x as i32, c.y as i32) {
                    continue;
                }
                self.sticks[i].planted = false;
            }
            if self.supported(world, i) {
                let s = &mut self.sticks[i];
                s.vy = 0.0;
                s.carry = 0.0;
                continue;
            }
            let wet = world.raw(c.x as i32, c.y as i32).kind() == Kind::Liquid;
            let s = &mut self.sticks[i];
            s.vy = if wet { (s.vy + GRAVITY * 0.3).min(MAX_SINK) } else { (s.vy + GRAVITY).min(MAX_FALL) };
            s.carry += s.vy;
            while self.sticks[i].carry >= 1.0 {
                self.sticks[i].carry -= 1.0;
                self.sticks[i].y += 1;
                if self.supported(world, i) {
                    self.sticks[i].vy = 0.0;
                    self.sticks[i].carry = 0.0;
                    break;
                }
            }
        }
        // Past the bottom of the world: gone.
        self.sticks.retain(|s| s.y < H() as i32 + 4);
    }

    /// While building nothing runs: a stick dropped in the air is put straight where it would land.
    pub fn settle(&mut self, world: &World) {
        for _ in 0..H() {
            if (0..self.sticks.len()).all(|i| self.sticks[i].planted || self.supported(world, i)) {
                break;
            }
            for s in &mut self.sticks {
                s.vy = MAX_FALL;
            }
            self.fall(world);
        }
        for s in &mut self.sticks {
            s.vy = 0.0;
            s.carry = 0.0;
        }
    }

    /// One step of the simulation: the sticks fall, the fuses burn, the burnt-out ones go off.
    /// Returns how many charges went off.
    pub fn step(&mut self, world: &mut World) -> usize {
        self.fall(world);
        for s in &mut self.sticks {
            s.fuse = s.fuse.saturating_sub(1);
        }
        let mut charges = 0;
        while let Some(first) = self.sticks.iter().position(|s| s.fuse == 0) {
            // The burnt-out stick and every stick touching it, touching them...: one charge.
            let mut group = vec![first];
            let mut k = 0;
            while k < group.len() {
                let g = self.sticks[group[k]].clone();
                for (j, o) in self.sticks.iter().enumerate() {
                    if !group.contains(&j) && g.touches(o) {
                        group.push(j);
                    }
                }
                k += 1;
            }
            group.sort_unstable();
            let fired: Vec<Stick> = group.iter().rev().map(|&j| self.sticks.remove(j)).collect();
            // The sticks lie over the matter they were pushed into: each one adds its own charge,
            // even where they overlap, and the blast breaks that matter like the rest.
            let mut room: Vec<(i32, i32)> = fired.iter().flat_map(|s| (s.y..s.y + THICK).flat_map(move |y| (s.x..s.x + LEN).map(move |x| (x, y)))).collect();
            room.sort_unstable();
            room.dedup();
            let c = fired.iter().fold(Vec2::ZERO, |a, s| a + s.centre()) / fired.len() as f32;
            world.set_off_sticks(c.x, c.y, (fired.len() as i32 * LEN * THICK) as f32, &room);
            charges += 1;
            // The shock sets off the sticks around, the nearest first.
            let reach = SYMPATHY * (fired.len() as f32).sqrt();
            for s in &mut self.sticks {
                let d = (s.centre() - c).length();
                if d < reach {
                    s.fuse = s.fuse.min(2 + (d / 10.0) as u32);
                }
            }
        }
        charges
    }

    pub fn draw(&self, view: View, burning: bool, time: f32) {
        for (i, s) in self.sticks.iter().enumerate() {
            draw_stick(view, vec2(s.x as f32, s.y as f32), s.fuse as f32 / FUSE as f32, burning, time + i as f32 * 1.7, 1.0);
        }
    }
}

/// A stick with its top left at `p` (world cells), `fuse` of its fuse left (1 = whole).
pub fn draw_stick(view: View, p: Vec2, fuse: f32, burning: bool, time: f32, alpha: f32) {
    let s = view.to_screen(p);
    let (w, h) = (LEN as f32 * view.s, THICK as f32 * view.s);
    let red = Color::from_rgba(196, 46, 38, (255.0 * alpha) as u8);
    let dark = Color::from_rgba(130, 26, 22, (255.0 * alpha) as u8);
    let paper = Color::from_rgba(226, 214, 190, (255.0 * alpha) as u8);
    draw_rectangle(s.x, s.y, w, h, red);
    draw_rectangle(s.x, s.y + h * 0.66, w, h * 0.34, dark);
    // The paper band with its label, and the crimped ends.
    draw_rectangle(s.x + w * 0.38, s.y, w * 0.24, h, paper);
    draw_rectangle(s.x, s.y, w * 0.06, h, dark);
    draw_rectangle(s.x + w * 0.94, s.y, w * 0.06, h, dark);
    // The fuse comes out of the right end and curls up, shorter as it burns.
    let len = (1.0 + 6.0 * fuse.clamp(0.0, 1.0)) * view.s;
    let base = vec2(s.x + w, s.y + h * 0.4);
    let mut prev = base;
    let n = 6;
    for k in 1..=n {
        let t = k as f32 / n as f32;
        let q = base + vec2(len * 0.55 * t, -len * t + (t * 3.0).sin() * view.s * 0.6);
        draw_line(prev.x, prev.y, q.x, q.y, (0.45 * view.s).max(1.0), Color::from_rgba(70, 60, 50, (255.0 * alpha) as u8));
        prev = q;
    }
    if burning {
        // A sputtering spark at the tip.
        let flick = 0.6 + 0.4 * (time * 37.0).sin().abs();
        draw_circle(prev.x, prev.y, view.s * (0.9 + 0.7 * flick), Color::new(1.0, 0.55, 0.1, 0.7 * alpha));
        draw_circle(prev.x, prev.y, view.s * 0.5 * flick, Color::new(1.0, 0.95, 0.6, alpha));
        for k in 0..3 {
            let a = time * 23.0 + k as f32 * 2.1;
            let r = view.s * (1.2 + (a * 1.3).sin().abs() * 1.6);
            draw_circle(prev.x + a.cos() * r, prev.y + a.sin() * r - view.s * 0.5, (0.3 * view.s).max(0.8), Color::new(1.0, 0.85, 0.3, alpha));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Mat;

    /// Stone from y = 300 down.
    fn ground() -> World {
        let mut w = World::new_static();
        for x in 0..W() as i32 {
            for y in 300..H() as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        w
    }

    fn run(d: &mut Dynamite, w: &mut World, steps: usize) -> usize {
        let mut n = 0;
        for _ in 0..steps {
            n += d.step(w);
            w.step();
        }
        n
    }

    #[test]
    fn a_stick_falls_through_air_and_water_and_rests_on_the_ground() {
        let mut w = ground();
        for x in 0..W() as i32 {
            for y in 260..300 {
                w.set(x, y, Mat::Water);
            }
        }
        let mut d = Dynamite::default();
        assert!(d.drop(&w, vec2(480.0, 100.0)));
        assert!(!d.sticks[0].planted);
        for _ in 0..170 {
            d.step(&mut w);
        }
        let s = &d.sticks[0];
        assert_eq!(s.y + THICK, 300, "it lies on the stone, under the water: {}", s.y);
    }

    #[test]
    fn a_stick_pushed_into_the_rock_stays_planted_and_blows_a_hole_from_inside() {
        let mut w = ground();
        let mut d = Dynamite::default();
        d.drop(&w, vec2(480.0, 320.0));
        assert!(d.sticks[0].planted);
        let y = d.sticks[0].y;
        for _ in 0..FUSE - 1 {
            assert_eq!(d.step(&mut w), 0);
        }
        assert_eq!(d.sticks[0].y, y, "it did not move");
        assert_eq!(w.raw(480, 320), Mat::Stone, "the stick lies over the rock, it does not replace it");
        assert_eq!(d.step(&mut w), 1, "the fuse is out");
        assert!(d.sticks.is_empty());
        assert!(!w.blasts.is_empty(), "it went off");
        assert!(w.raw(480, 320) != Mat::Stone, "a hole where the stick was");
        // The rock around is broken from the inside, well past the stick itself.
        let broken = |x0: i32, x1: i32, y0: i32, y1: i32| (x0..x1).map(|x| (y0..y1).filter(|&y| w.raw(x, y) != Mat::Stone).count()).sum::<usize>();
        assert!(broken(470, 490, 312, 330) > 300, "crushed round the stick: {}", broken(470, 490, 312, 330));
        assert!(broken(380, 580, 300, 420) > 800, "and cracked further out: {}", broken(380, 580, 300, 420));
        assert!(broken(380, 580, 360, 420) < broken(380, 580, 300, 360), "less and less far from it");
    }

    /// Cells of rock broken by `n` sticks planted in the same hole.
    fn planted(n: usize) -> usize {
        let mut w = ground();
        let mut d = Dynamite::default();
        for _ in 0..n {
            d.drop(&w, vec2(480.0, 380.0));
        }
        assert!(d.sticks.iter().all(|s| s.planted && s.x == d.sticks[0].x && s.y == d.sticks[0].y));
        run(&mut d, &mut w, FUSE as usize + 2);
        (280..680).map(|x| (300..H() as i32).filter(|&y| w.raw(x, y) != Mat::Stone).count()).sum()
    }

    #[test]
    fn sticks_planted_in_the_same_hole_add_up() {
        // They lie one over the other: every one of them adds its own charge.
        let (one, four) = (planted(1), planted(4));
        eprintln!("rock broken by one stick {one}, by four in the same hole {four}");
        assert!(four > one * 2, "{one} vs {four}");
    }

    #[test]
    fn the_shock_sets_off_a_stick_nearby_a_few_steps_later() {
        let mut w = ground();
        let mut d = Dynamite::default();
        d.drop(&w, vec2(470.0, 380.0));
        d.drop(&w, vec2(495.0, 380.0));
        d.sticks[1].fuse = FUSE * 2;
        let mut went: Vec<usize> = vec![];
        for step in 0..FUSE as usize + 30 {
            if d.step(&mut w) > 0 {
                went.push(step);
            }
        }
        assert_eq!(went.len(), 2, "two charges, one after the other: {went:?}");
        let delay = went[1] - went[0];
        assert!((2..=8).contains(&delay), "the second goes off when the front reaches it, not at once nor on its fuse: {delay}");
    }

    #[test]
    fn building_puts_a_dropped_stick_where_it_lands() {
        let w = ground();
        let mut d = Dynamite::default();
        d.drop(&w, vec2(200.0, 50.0));
        d.drop(&w, vec2(200.0, 50.0));
        d.settle(&w);
        assert_eq!(d.sticks[0].y + THICK, 300);
        assert_eq!(d.sticks[1].y + THICK, d.sticks[0].y, "the second lies on the first");
        assert!(d.sticks.iter().all(|s| s.fuse == FUSE), "the fuses are not lit while building");
    }

    #[test]
    fn sticks_that_touch_go_off_as_one_charge_and_the_shock_sets_off_the_others() {
        // A heap of three sticks, and one alone a little way off, dropped later.
        let mut w = ground();
        let mut d = Dynamite::default();
        for _ in 0..3 {
            d.drop(&w, vec2(480.0, 200.0));
        }
        d.drop(&w, vec2(500.0, 200.0));
        d.settle(&w);
        d.sticks[3].fuse = FUSE * 3;
        let mut steps = 0;
        let mut charges = 0;
        while !d.sticks.is_empty() && steps < FUSE as usize + 40 {
            charges += d.step(&mut w);
            steps += 1;
        }
        assert!(d.sticks.is_empty(), "all went off");
        assert!(steps < FUSE as usize + 12, "the far stick went off with the heap, not on its own fuse: {steps}");
        assert!(charges <= 2, "the heap is one charge: {charges}");

        // More sticks, a bigger hole.
        let hole = |n: usize| {
            let mut w = ground();
            let mut d = Dynamite::default();
            for _ in 0..n {
                d.drop(&w, vec2(480.0, 200.0));
            }
            d.settle(&w);
            run(&mut d, &mut w, FUSE as usize + 300);
            (380..580).map(|x| (300..420).filter(|&y| w.raw(x, y) != Mat::Stone).count()).sum::<usize>()
        };
        let (one, five) = (hole(1), hole(5));
        eprintln!("hole of one stick {one}, of five {five}");
        assert!(five > one, "{one} vs {five}");
    }
}

