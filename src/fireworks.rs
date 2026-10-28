//! Just for fun: gunpowder that goes off at night sends up a firework. Rockets climb out of the
//! blast and burst into showers of coloured sparks. Like real fireworks, the colours come from
//! metal salts mixed into the powder: copper burns blue, strontium red, sodium yellow, barium green.
//! Nothing here touches the world: it is all light in the sky.

use crate::render::View;
use crate::gfx::*;

/// The pull the show was drawn for, cells per second²; under the world's gravity
/// (`units::g_cells`) it plays the same shapes faster (see `pace`).
const DRAWN_G: f32 = 90.0;

/// How much faster the show goes than it was drawn: speeds up by this, times down by it, so that
/// every rocket and spark follows the same path under the world's gravity.
fn pace() -> f32 {
    (crate::world::units::g_cells() / DRAWN_G).sqrt()
}
/// At most this many rockets in the air, and sparks on screen.
const MAX_ROCKETS: usize = 24;
const MAX_SPARKS: usize = 2500;
/// Seconds between two salvos, however much powder goes off at once.
const SALVO_GAP: f32 = 0.35;

/// The metal salts and the colours they burn with.
pub const SALTS: [(&str, (u8, u8, u8)); 6] = [
    ("strontium", (255, 60, 70)),
    ("cuivre", (70, 150, 255)),
    ("sodium", (255, 210, 60)),
    ("baryum", (90, 240, 110)),
    ("calcium", (255, 140, 40)),
    ("magnésium", (255, 255, 255)),
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// A round ball of sparks.
    Peony,
    /// A flat ring.
    Ring,
    /// Long golden trails that droop like a willow.
    Willow,
}

#[derive(Clone, Debug)]
pub struct Rocket {
    pub pos: Vec2,
    pub vel: Vec2,
    /// Seconds left before it bursts.
    pub fuse: f32,
    pub color: Color,
    pub shape: Shape,
}

#[derive(Clone, Debug)]
pub struct Spark {
    pub pos: Vec2,
    pub vel: Vec2,
    pub age: f32,
    pub life: f32,
    pub color: Color,
    /// Willow sparks glitter and hang in the air longer.
    pub drag: f32,
}

#[derive(Clone, Default)]
pub struct Fireworks {
    pub rockets: Vec<Rocket>,
    pub sparks: Vec<Spark>,
    cooldown: f32,
    rng: u32,
    /// Bursts since the last call to `take_bursts` (for the sound).
    bursts: u32,
}

impl Fireworks {
    pub fn new() -> Self {
        Fireworks { rng: 0xf1e2_0a0b, ..Default::default() }
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

    pub fn active(&self) -> bool {
        !self.rockets.is_empty() || !self.sparks.is_empty()
    }

    /// Powder went off at `at` (world cells): send up a salvo of `n` rockets, unless one has just
    /// left. Returns whether rockets were launched.
    pub fn launch(&mut self, at: Vec2, n: usize) -> bool {
        if self.cooldown > 0.0 || self.rockets.len() >= MAX_ROCKETS {
            return false;
        }
        self.cooldown = SALVO_GAP;
        for _ in 0..n.clamp(1, MAX_ROCKETS - self.rockets.len()) {
            let salt = SALTS[(self.rand() * SALTS.len() as f32) as usize % SALTS.len()].1;
            let shape = match (self.rand() * 3.0) as u32 {
                0 => Shape::Ring,
                1 => Shape::Willow,
                _ => Shape::Peony,
            };
            let color = if shape == Shape::Willow { Color::from_rgba(255, 196, 90, 255) } else { Color::from_rgba(salt.0, salt.1, salt.2, 255) };
            let vel = vec2(self.between(-40.0, 40.0), -self.between(170.0, 230.0)) * pace();
            let fuse = self.between(1.0, 1.6) / pace();
            self.rockets.push(Rocket { pos: at, vel, fuse, color, shape });
        }
        true
    }

    /// How many rockets burst since the last call.
    pub fn take_bursts(&mut self) -> u32 {
        std::mem::take(&mut self.bursts)
    }

    pub fn update(&mut self, dt: f32) {
        self.cooldown = (self.cooldown - dt).max(0.0);
        let mut burst = vec![];
        let mut trail = vec![];
        for r in &mut self.rockets {
            r.vel.y += crate::world::units::g_cells() * dt;
            r.pos += r.vel * dt;
            r.fuse -= dt;
            trail.push(r.pos);
            if r.fuse <= 0.0 {
                burst.push(r.clone());
            }
        }
        self.rockets.retain(|r| r.fuse > 0.0);
        // The rocket's own glowing trail.
        for p in trail {
            if self.sparks.len() < MAX_SPARKS {
                let jitter = vec2(self.between(-8.0, 8.0), self.between(0.0, 15.0)) * pace();
                self.sparks.push(Spark { pos: p, vel: jitter, age: 0.0, life: 0.35 / pace(), color: Color::from_rgba(255, 200, 120, 255), drag: 2.0 * pace() });
            }
        }
        for r in burst {
            self.burst(&r);
        }
        for s in &mut self.sparks {
            s.age += dt;
            let k = (-s.drag * dt).exp();
            s.vel *= k;
            s.vel.y += crate::world::units::g_cells() * 0.45 * dt;
            s.pos += s.vel * dt;
        }
        self.sparks.retain(|s| s.age < s.life);
    }

    fn burst(&mut self, r: &Rocket) {
        self.bursts += 1;
        let n = match r.shape {
            Shape::Peony => 90,
            Shape::Ring => 60,
            Shape::Willow => 70,
        };
        // Now and then a second colour in the same burst, as in a real double-salt shell.
        let second = if self.rand() < 0.35 {
            let s = SALTS[(self.rand() * SALTS.len() as f32) as usize % SALTS.len()].1;
            Some(Color::from_rgba(s.0, s.1, s.2, 255))
        } else {
            None
        };
        for k in 0..n {
            if self.sparks.len() >= MAX_SPARKS {
                break;
            }
            let a = k as f32 / n as f32 * std::f32::consts::TAU + self.between(-0.05, 0.05);
            let (speed, life, drag) = match r.shape {
                Shape::Peony => (self.between(70.0, 150.0), self.between(1.2, 1.8), 1.6),
                Shape::Ring => (135.0 + self.between(-5.0, 5.0), self.between(1.1, 1.5), 1.5),
                Shape::Willow => (self.between(50.0, 110.0), self.between(2.2, 3.0), 2.4),
            };
            let color = match second {
                Some(c) if k % 2 == 1 => c,
                _ => r.color,
            };
            let vel = vec2(a.cos(), a.sin()) * speed * pace() + r.vel * 0.3;
            self.sparks.push(Spark { pos: r.pos, vel, age: 0.0, life: life / pace(), color, drag: drag * pace() });
        }
    }

    pub fn draw(&self, view: View, time: f32) {
        for r in &self.rockets {
            let p = view.to_screen(r.pos);
            draw_circle(p.x, p.y, (1.6 * view.s).max(1.5), Color::from_rgba(255, 236, 200, 255));
        }
        for (i, s) in self.sparks.iter().enumerate() {
            let p = view.to_screen(s.pos);
            let fade = 1.0 - s.age / s.life;
            // Sparks twinkle as they die out.
            let twinkle = if fade < 0.4 { 0.5 + 0.5 * (time * 30.0 + i as f32).sin().abs() } else { 1.0 };
            let a = fade * twinkle;
            let r = (1.4 * view.s).max(1.5);
            draw_circle(p.x, p.y, r * 3.0, Color::new(s.color.r, s.color.g, s.color.b, a * 0.22));
            draw_circle(p.x, p.y, r, Color::new(s.color.r, s.color.g, s.color.b, a));
            draw_circle(p.x, p.y, r * 0.45, Color::new(1.0, 1.0, 1.0, a * 0.9));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(f: &mut Fireworks, seconds: f32) {
        let mut t = 0.0;
        while t < seconds {
            f.update(1.0 / 60.0);
            t += 1.0 / 60.0;
        }
    }

    #[test]
    fn a_rocket_climbs_bursts_into_sparks_and_the_sky_clears() {
        let mut f = Fireworks::new();
        assert!(f.launch(vec2(400.0, 400.0), 3));
        assert_eq!(f.rockets.len(), 3);
        // (The show goes at `pace`: the times are the ones it was drawn with.)
        run(&mut f, 0.8 / pace());
        assert!(f.rockets.iter().all(|r| r.pos.y < 300.0), "rockets climb high above the blast");
        run(&mut f, 1.0 / pace());
        assert!(f.rockets.is_empty(), "every rocket has burst");
        assert_eq!(f.take_bursts(), 3);
        assert!(f.sparks.len() > 150, "a shower of sparks: {}", f.sparks.len());
        let high = f.sparks.iter().filter(|s| s.pos.y < 300.0).count();
        assert!(high > 100, "the bursts are up in the sky: {high}");
        run(&mut f, 4.0);
        assert!(!f.active(), "and then it is dark again");
    }

    #[test]
    fn a_big_pile_of_powder_does_not_flood_the_sky() {
        let mut f = Fireworks::new();
        let mut launched = 0;
        for k in 0..600 {
            if f.launch(vec2(200.0 + k as f32, 400.0), 4) {
                launched += 1;
            }
            f.update(1.0 / 60.0);
            assert!(f.rockets.len() <= MAX_ROCKETS && f.sparks.len() <= MAX_SPARKS);
        }
        // Ten seconds of explosions: a salvo every third of a second or so, not one per grain.
        assert!((10..=40).contains(&launched), "{launched} salvos");
    }
}
