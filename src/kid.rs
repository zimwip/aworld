//! Just for fun: now and then a little boy comes for a walk in the world. Short blond hair, a
//! yellow hoodie with a green dinosaur on it, blue joggers and dark socks, and a big grin. He walks
//! along the ground, jumps over walls, turns cartwheels and somersaults, and says what he thinks
//! of what he sees in a cartoon speech bubble. He is a drawing over the world: nothing burns him or
//! blows him up, but if something hits him, or someone clicks on him, he lets you know.

use crate::render::View;
use crate::ui::{rrect, rrect_lines, Ui};
use crate::world::{Kind, Mat, World, H, W};
use macroquad::prelude::*;
use std::f32::consts::TAU;

/// The sprite grid, and the size of one of its pixels in world cells.
const SW: usize = 16;
const SH: usize = 24;
const PIX: f32 = 1.3;
/// From the soles to the top of the hair, in cells.
pub const HEIGHT: f32 = 22.0 * PIX;
/// Walking speed (cells per second) and pull of gravity (cells per second squared).
const SPEED: f32 = 14.0;
const GRAVITY: f32 = 320.0;
/// He climbs steps this high, jumps walls up to this high, and turns round before higher ones.
const STEP: i32 = 4;
const MAX_JUMP: i32 = 20;
/// Seconds between visits, at random, and how long a visit lasts before he heads home.
const GAP: (f32, f32) = (45.0, 120.0);
const VISIT: (f32, f32) = (45.0, 75.0);
/// A cartwheel takes this long.
const WHEEL: f32 = 0.9;
/// Seconds between two "ouch", so one fire does not make him shout every frame.
const OUCH_REST: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    Walk,
    /// Standing still for a moment (waving while he talks).
    Idle,
    Jump,
    /// A somersault in the air.
    Flip,
    Cartwheel,
    Hurt,
    /// Going home: one big jump up out of the picture.
    Leap,
}

/// What hit him.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Click,
    Burn,
    Blast,
    Block,
    Car,
    Sand,
    Hail,
    Buried,
}

/// What there is around him besides the matter of the world, for his remarks.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sights {
    pub whale: bool,
    pub plane: bool,
    pub truck: bool,
    pub fireworks: bool,
    pub cars: usize,
    pub blocks: usize,
    pub night: bool,
    pub temp_c: f32,
}

#[derive(Clone, Debug)]
pub struct Bubble {
    pub text: String,
    pub age: f32,
    pub life: f32,
    /// A shout: drawn with a red outline.
    pub ouch: bool,
}

#[derive(Clone, Debug)]
pub struct Kid {
    /// Middle of his feet: x, and the row of the ground under them.
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    /// +1 walking right, -1 left.
    pub dir: f32,
    pub air: bool,
    pub act: Act,
    /// Seconds in the current act.
    pub act_t: f32,
    /// Spin of the body (radians), for cartwheels and somersaults.
    pub angle: f32,
    pub age: f32,
    /// When he heads home.
    pub stay: f32,
    pub stride: f32,
    pub bubble: Option<Bubble>,
    talk_in: f32,
    trick_in: f32,
    ouch_in: f32,
    /// Clicks on him lately, for answers that get crosser.
    pokes: u32,
    poke_t: f32,
    /// Highest row while falling, to notice big falls.
    fall_from: f32,
    said_swim: bool,
    last_trick: Act,
}

impl Kid {
    /// The middle of his body, where he spins round.
    pub fn centre(&self) -> Vec2 {
        if self.angle == 0.0 {
            return vec2(self.x, self.y - SH as f32 / 2.0 * PIX);
        }
        // Spinning, whatever is lowest (his hands, his feet or his head) touches the ground.
        let (arms, legs, face) = pose(self, 0.0);
        let g = sprite(arms, legs, face, 0);
        let (s, c) = self.angle.sin_cos();
        let mut low: f32 = 0.0;
        for (j, row) in g.iter().enumerate() {
            for (i, &px) in row.iter().enumerate() {
                if px != b'.' {
                    let (lx, ly) = (i as f32 + 0.5 - SW as f32 / 2.0, j as f32 + 0.5 - SH as f32 / 2.0);
                    low = low.max(lx * s + ly * c + 0.5);
                }
            }
        }
        vec2(self.x, self.y - low * PIX)
    }

    /// Is `p` (world) on him? A little generous, so a small boy is easy to click.
    pub fn contains(&self, p: Vec2) -> bool {
        let c = self.centre();
        (p.x - c.x).abs() < 8.0 * PIX && (p.y - c.y).abs() < 13.0 * PIX
    }

    fn say(&mut self, text: &str, ouch: bool) {
        let life = 2.6 + text.chars().count() as f32 * 0.045;
        self.bubble = Some(Bubble { text: text.to_string(), age: 0.0, life, ouch });
    }

    /// Something hit him (`roll`, 0..1, picks what he says).
    fn ouch(&mut self, hit: Hit, roll: f32) {
        let k = self;
        if hit == Hit::Click {
            k.pokes = if k.poke_t > 0.0 { k.pokes + 1 } else { 1 };
            k.poke_t = 6.0;
        } else if k.ouch_in > 0.0 {
            return;
        }
        k.ouch_in = OUCH_REST;
        let pick = |lines: &[&'static str]| lines[((roll * lines.len() as f32) as usize).min(lines.len() - 1)];
        let line = match hit {
            Hit::Click => match k.pokes {
                1 => pick(&["Aïe !", "Ouille !", "Hé ! Ça chatouille !"]),
                2 => "Hé ho, ça va pas non ?!",
                3 => "Arrête de me cliquer dessus !",
                _ => "Bon, puisque c'est comme ça, je rentre à la maison. Na !",
            },
            Hit::Burn => pick(&["Aïe, ça brûle !", "Ouille ouille, chaud chaud chaud !", "Aïe ! Heureusement, mon sweat est ignifugé."]),
            Hit::Blast => pick(&["BOUM ! Hé ho, ça va pas non ?!", "Aïe ! J'ai les oreilles qui sifflent !"]),
            Hit::Block => pick(&["Aïe ! Ça tombe sur la tête !", "Hé ho ! Attention avec tes blocs !"]),
            Hit::Car => pick(&["Hé ho ! Regarde où tu roules !", "Aïe ! Il a pas vu le piéton !"]),
            Hit::Sand => pick(&["Hé ! Qui m'a jeté du sable ?", "Pouah ! J'en ai plein les cheveux !"]),
            Hit::Hail => pick(&["Aïe ! La grêle !", "Ouille, ça tape sur la tête !"]),
            Hit::Buried => "Hé ho ! Je suis pas un trésor à enterrer !",
        };
        k.say(line, true);
        if hit == Hit::Click && k.pokes >= 4 {
            k.stay = 0.0;
        }
        if k.act != Act::Leap {
            k.act = Act::Hurt;
            k.act_t = 0.0;
            k.angle = 0.0;
            if !k.air {
                k.air = true;
                k.vy = -75.0;
                k.vx = -k.dir * 12.0;
                k.fall_from = k.y;
            }
        }
    }

    fn talking(&self) -> bool {
        self.bubble.as_ref().map_or(false, |b| b.age < b.life)
    }
}

#[derive(Default)]
pub struct Kids {
    pub kid: Option<Kid>,
    cooldown: f32,
    rng: u32,
    /// The last few topics he talked about, so he does not repeat himself.
    recent: Vec<Topic>,
}

fn floor(world: &World, x: i32, y: i32) -> bool {
    let x = x.clamp(0, W as i32 - 1);
    world.hard_for_car(x, y) || world.at(x, y).kind() == Kind::Liquid
}

/// The ground he can stand on in column `x` near row `y`: the first floor cell with room above it,
/// from `up` cells above to `down` cells below.
fn ground_near(world: &World, x: i32, y: i32, up: i32, down: i32) -> Option<i32> {
    ((y - up)..=(y + down)).find(|&gy| floor(world, x, gy) && !floor(world, x, gy - 1))
}

/// The ground seen from the sky over column `x` (None over water or a bottomless hole).
fn ground_from_top(world: &World, x: i32) -> Option<i32> {
    let x = x.clamp(0, W as i32 - 1);
    (0..H as i32).find(|&y| floor(world, x, y)).filter(|&y| world.at(x, y).kind() != Kind::Liquid)
}

impl Kids {
    pub fn new() -> Self {
        Kids { cooldown: 25.0, rng: 0x0d1_0ba5e, ..Default::default() }
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

    fn pick<'a>(&mut self, lines: &[&'a str]) -> &'a str {
        lines[((self.rand() * lines.len() as f32) as usize).min(lines.len() - 1)]
    }

    /// He walks in from a side of the world that has ground, or drops in from the sky if
    /// neither has. Returns whether he came.
    pub fn summon(&mut self, world: &World) -> bool {
        if self.kid.is_some() {
            return false;
        }
        let left_first = self.rand() < 0.5;
        let mut start = None;
        for left in [left_first, !left_first] {
            let edge = if left { 0 } else { W as i32 - 1 };
            if let Some(y) = ground_from_top(world, edge) {
                let x = if left { -10.0 } else { W as f32 + 10.0 };
                start = Some((x, y as f32, if left { 1.0 } else { -1.0 }, false));
                break;
            }
        }
        if start.is_none() {
            for _ in 0..20 {
                let x = self.between(60.0, W as f32 - 60.0);
                if ground_from_top(world, x as i32).is_some() {
                    start = Some((x, -30.0, if self.rand() < 0.5 { 1.0 } else { -1.0 }, true));
                    break;
                }
            }
        }
        let Some((x, y, dir, air)) = start else { return false };
        let stay = self.between(VISIT.0, VISIT.1);
        self.cooldown = self.between(GAP.0, GAP.1);
        let mut kid = Kid {
            x,
            y,
            vx: 0.0,
            vy: 0.0,
            dir,
            air,
            act: if air { Act::Jump } else { Act::Walk },
            act_t: 0.0,
            angle: 0.0,
            age: 0.0,
            stay,
            stride: 0.0,
            bubble: None,
            talk_in: self.between(6.0, 10.0),
            trick_in: self.between(4.0, 9.0),
            ouch_in: 0.0,
            pokes: 0,
            poke_t: 0.0,
            fall_from: y,
            said_swim: false,
            last_trick: Act::Walk,
        };
        let hello = if air { "Youhouuu ! Attention, j'arrive !" } else { self.pick(HELLO) };
        kid.say(hello, false);
        self.kid = Some(kid);
        true
    }

    /// For screenshots: the boy standing at `x`, in the middle of `act` (walk, wave, jump, flip,
    /// wheel, ouch), saying something.
    pub fn demo(&mut self, world: &World, x: f32, act: &str) {
        self.kid = None;
        if !self.summon(world) {
            return;
        }
        let k = self.kid.as_mut().unwrap();
        k.x = x;
        k.y = ground_from_top(world, x as i32).unwrap_or(400) as f32;
        k.air = false;
        k.age = 5.0;
        (k.stay, k.talk_in, k.trick_in) = (1000.0, 1000.0, 1000.0);
        k.dir = 1.0;
        let hop = |k: &mut Kid, act: Act, up: f32| {
            k.act = act;
            k.air = true;
            k.y -= up;
            k.vy = -30.0;
        };
        match act {
            "wave" => {
                k.act = Act::Idle;
                k.act_t = -1000.0;
                k.say("Coucou ! Tu construis quoi aujourd'hui ?", false);
            }
            "jump" => {
                hop(k, Act::Jump, 14.0);
                k.say("Youpi !", false);
            }
            "flip" => {
                hop(k, Act::Flip, 16.0);
                k.angle = TAU * 0.3;
                k.say("Regardez ! BRAVE MOVES !", false);
            }
            "wheel" => {
                k.act = Act::Cartwheel;
                k.act_t = WHEEL * 0.2;
                k.angle = TAU * 0.2;
                k.say("Je suis le roi de la roue !", false);
            }
            "ouch" => {
                self.ouch(Hit::Click);
                self.ouch(Hit::Click);
            }
            _ => {
                let line = self.remark(&self.kid.clone().unwrap(), world, &Sights { temp_c: 15.0, ..Default::default() });
                self.kid.as_mut().unwrap().say(&line, false);
            }
        }
    }

    /// Something hit him, or someone clicked on him.
    pub fn ouch(&mut self, hit: Hit) {
        let roll = self.rand();
        if let Some(k) = &mut self.kid {
            k.ouch(hit, roll);
        }
    }

    /// `dt` in seconds.
    pub fn update(&mut self, dt: f32, world: &World, sights: &Sights) {
        let Some(mut k) = self.kid.take() else {
            self.cooldown -= dt;
            if self.cooldown <= 0.0 {
                let came = self.summon(world);
                self.cooldown = if came { self.between(GAP.0, GAP.1) } else { 15.0 };
            }
            return;
        };
        k.age += dt;
        k.act_t += dt;
        k.ouch_in -= dt;
        k.poke_t -= dt;
        if let Some(b) = &mut k.bubble {
            b.age += dt;
            if b.age > b.life + 0.3 {
                k.bubble = None;
            }
        }
        self.feel(&mut k, world, dt);
        if k.age > k.stay && !matches!(k.act, Act::Leap | Act::Hurt) && !k.air {
            self.go_home(&mut k);
        }
        match k.act {
            Act::Leap => {
                k.y += k.vy * dt;
                k.x += k.vx * dt;
                k.angle += k.dir * TAU * 1.5 * dt;
                if k.y < -60.0 {
                    return;
                }
            }
            _ if k.air => self.fly(&mut k, world, dt),
            Act::Cartwheel => {
                k.angle = k.dir * TAU * (k.act_t / WHEEL).min(1.0);
                if !self.walk(&mut k, world, dt, 1.7) || k.act_t >= WHEEL {
                    k.angle = 0.0;
                    k.act = Act::Walk;
                    k.act_t = 0.0;
                    if self.rand() < 0.5 && !k.talking() {
                        let l = self.pick(WHEELED);
                        k.say(l, false);
                    }
                }
            }
            Act::Idle => {
                if k.act_t > 1.6 {
                    k.act = Act::Walk;
                    k.act_t = 0.0;
                }
            }
            _ => {
                k.act = Act::Walk;
                self.walk(&mut k, world, dt, 1.0);
                k.trick_in -= dt;
                if k.trick_in <= 0.0 && k.age < k.stay {
                    self.trick(&mut k, world);
                }
            }
        }
        // Gone off a side of the world on his way home, or fell out of it.
        if (k.age > k.stay && (k.x < -14.0 || k.x > W as f32 + 14.0)) || k.y > H as f32 + 40.0 {
            return;
        }
        k.talk_in -= dt;
        if k.talk_in <= 0.0 && !k.talking() && k.act != Act::Leap {
            let line = self.remark(&k, world, sights);
            k.say(&line, false);
            k.talk_in = self.between(7.0, 14.0);
            // Now and then he stops to say it, and waves.
            if k.act == Act::Walk && !k.air && self.rand() < 0.4 {
                k.act = Act::Idle;
                k.act_t = 0.0;
            }
        }
        self.kid = Some(k);
    }

    /// He walks along the ground. Returns false when he had to stop or turn round.
    fn walk(&mut self, k: &mut Kid, world: &World, dt: f32, pace: f32) -> bool {
        let leaving = k.age > k.stay;
        // Out of the world (coming in, or going home), the edge row is the ground.
        let col = |x: f32| (x as i32).clamp(0, W as i32 - 1);
        let in_liquid = world.at(col(k.x), k.y as i32).kind() == Kind::Liquid;
        let speed = SPEED * pace * if in_liquid { 0.5 } else { 1.0 };
        k.stride += speed * dt;
        // Not at the edges of the world, except on the way in or out.
        let ahead_x = k.x + k.dir * 3.0;
        let coming_in = k.age < 3.0 && (k.x < 0.0 || k.x > W as f32);
        if !leaving && !coming_in && (ahead_x < 8.0 || ahead_x > W as f32 - 8.0) {
            k.dir = -k.dir;
            return false;
        }
        let ax = col(ahead_x);
        let y = k.y.round() as i32;
        match ground_near(world, ax, y, STEP, 3) {
            Some(g) => {
                // Water ahead: he does not go in on his own.
                if world.at(ax, g).kind() == Kind::Liquid && !in_liquid {
                    k.dir = -k.dir;
                    if !k.talking() && self.rand() < 0.5 {
                        let l = self.pick(NO_SWIM);
                        k.say(l, false);
                    }
                    return false;
                }
                k.x += k.dir * speed * dt;
                k.y += (g as f32 - k.y) * (dt * 14.0).min(1.0);
                true
            }
            None => {
                let wall = (1..=STEP + 1).any(|d| floor(world, ax, y - d));
                if wall {
                    // A wall: jump it if he can, or turn round.
                    match ground_near(world, ax, y, MAX_JUMP, 0) {
                        Some(top) if k.act != Act::Cartwheel => {
                            let h = (y - top) as f32 + 4.0;
                            k.vy = -(2.0 * GRAVITY * h).sqrt();
                            k.vx = k.dir * SPEED;
                            k.air = true;
                            k.act = Act::Jump;
                            k.act_t = 0.0;
                            k.fall_from = k.y;
                        }
                        _ => k.dir = -k.dir,
                    }
                } else {
                    // The ground drops away: off he goes.
                    k.air = true;
                    k.vx = k.dir * speed;
                    k.vy = 0.0;
                    k.fall_from = k.y;
                    k.angle = 0.0;
                    if k.act == Act::Cartwheel {
                        k.act = Act::Jump;
                    }
                }
                false
            }
        }
    }

    /// In the air: jumping, flipping, falling.
    fn fly(&mut self, k: &mut Kid, world: &World, dt: f32) {
        k.vy += GRAVITY * dt;
        let (nx, ny) = (k.x + k.vx * dt, k.y + k.vy * dt);
        // He bumps into walls (not on the way up: he jumps over them) and ceilings.
        if k.vy >= 0.0 && (2..14).any(|d| floor(world, nx as i32 + k.dir as i32 * 3, ny as i32 - d)) && (0.0..W as f32).contains(&nx) {
            k.vx = 0.0;
        } else {
            k.x = nx;
        }
        if k.vy < 0.0 && floor(world, k.x as i32, ny as i32 - HEIGHT as i32) && (0.0..W as f32).contains(&k.x) {
            k.vy = 0.0;
        }
        if k.act == Act::Flip {
            // One whole turn over the time of the jump.
            k.angle = (k.angle + k.dir * TAU * dt / 0.72).clamp(-TAU, TAU);
        }
        let landed = if k.vy > 0.0 {
            let x = (k.x as i32).clamp(0, W as i32 - 1);
            ((k.y as i32)..=(ny as i32).max(k.y as i32)).find(|&gy| floor(world, x, gy) && !floor(world, x, gy - 1))
        } else {
            None
        };
        match landed {
            Some(g) => {
                k.y = g as f32;
                k.vy = 0.0;
                k.vx = 0.0;
                k.air = false;
                let was = k.act;
                k.angle = 0.0;
                k.act = Act::Walk;
                k.act_t = 0.0;
                if k.y - k.fall_from > 70.0 && !k.talking() {
                    k.say("Wouaaah ! ... Même pas mal !", false);
                } else if was == Act::Flip && !k.talking() && self.rand() < 0.6 {
                    let l = self.pick(FLIPPED);
                    k.say(l, false);
                }
            }
            None => k.y = ny,
        }
    }

    /// Now and then, on flat ground, a jump, a cartwheel or a somersault.
    fn trick(&mut self, k: &mut Kid, world: &World) {
        k.trick_in = self.between(5.0, 12.0);
        // Room ahead: flat ground and nothing in the way, so he does not cartwheel into a wall.
        let y = k.y as i32;
        let flat = (4..=30).step_by(4).all(|d| {
            let x = (k.x + k.dir * d as f32) as i32;
            (0..W as i32).contains(&x) && ground_near(world, x, y, 2, 2).map_or(false, |g| world.at(x, g).kind() != Kind::Liquid) && !(3..=24).any(|h| floor(world, x, y - h))
        });
        if !flat {
            return;
        }
        // Never the same trick twice in a row.
        const TRICKS: [Act; 3] = [Act::Cartwheel, Act::Flip, Act::Jump];
        let r = self.rand();
        let mut i = if r < 0.4 { 0 } else if r < 0.8 { 1 } else { 2 };
        if TRICKS[i] == k.last_trick {
            i = (i + 1) % TRICKS.len();
        }
        k.last_trick = TRICKS[i];
        k.act_t = 0.0;
        k.fall_from = k.y;
        if i == 0 {
            k.act = Act::Cartwheel;
        } else if i == 1 {
            k.act = Act::Flip;
            k.air = true;
            k.vy = -(GRAVITY * 0.72 / 2.0);
            k.vx = k.dir * SPEED * 0.8;
            if !k.talking() && self.rand() < 0.4 {
                k.say("Regardez ! BRAVE MOVES !", false);
            }
        } else {
            k.act = Act::Jump;
            k.air = true;
            k.vy = -150.0;
            k.vx = k.dir * SPEED;
        }
    }

    fn go_home(&mut self, k: &mut Kid) {
        // He walks to the nearer side; if he is stuck far from it, he leaps away.
        let home = if k.x < W as f32 / 2.0 { -1.0 } else { 1.0 };
        if k.age > k.stay + 25.0 || k.stay == 0.0 {
            k.act = Act::Leap;
            k.act_t = 0.0;
            k.vy = -260.0;
            k.vx = home * 40.0;
            k.dir = home;
            if !k.talking() {
                let l = self.pick(BYE);
                k.say(l, false);
            }
            return;
        }
        if k.dir != home && k.act == Act::Walk {
            k.dir = home;
            if !k.talking() {
                let l = self.pick(BYE);
                k.say(l, false);
            }
        }
    }

    /// What touches him: fire and lava, things falling on his head, being buried, water.
    fn feel(&mut self, k: &mut Kid, world: &World, dt: f32) {
        let c = k.centre();
        let (mut hot, mut grit, mut hail, mut solid, mut wet) = (0, 0, 0, 0, 0);
        for dy in (-10..=10).step_by(2) {
            for dx in (-4..=4).step_by(2) {
                let (x, y) = (c.x as i32 + dx, c.y as i32 + dy);
                let m = world.at(x, y);
                match m {
                    Mat::Fire | Mat::Lava | Mat::Ember | Mat::Molten | Mat::MoltenSteel => hot += 1,
                    Mat::Hail => hail += 1,
                    Mat::Sand | Mat::Gunpowder | Mat::Ash | Mat::Rust | Mat::Charcoal | Mat::Saltpeter | Mat::Shard if dy < -4 => grit += 1,
                    Mat::Water if dy < -4 => wet += 1,
                    _ => {}
                }
                if dy < 0 && world.hard_for_car(x, y) {
                    solid += 1;
                }
            }
        }
        let submerged = world.at(k.x as i32, k.y as i32 - 3).kind() == Kind::Liquid && world.at(k.x as i32, k.y as i32 - 1).kind() == Kind::Liquid;
        if submerged && !k.air {
            // He floats up to the surface.
            k.y -= 30.0 * dt;
            if !k.said_swim && !k.talking() {
                k.said_swim = true;
                k.say("Glou glou ! Heureusement, je sais nager !", false);
            }
        }
        // Buried: he climbs out the top.
        if solid >= 12 && !k.air {
            k.y -= 40.0 * dt;
        }
        let hit = if hot > 0 {
            Some(Hit::Burn)
        } else if solid >= 20 {
            Some(Hit::Buried)
        } else if grit >= 2 {
            Some(Hit::Sand)
        } else if hail >= 1 {
            Some(Hit::Hail)
        } else {
            None
        };
        if let Some(h) = hit {
            if k.ouch_in <= 0.0 {
                if h == Hit::Burn {
                    // He runs away from the heat.
                    k.dir = -k.dir;
                }
                let roll = self.rand();
                k.ouch(h, roll);
            }
        } else if wet >= 3 && !k.talking() && k.talk_in > 2.0 && !submerged {
            let l = self.pick(RAIN);
            k.say(l, false);
            k.talk_in = self.between(10.0, 16.0);
        }
    }

    /// Something to say about what he sees around him.
    fn remark(&mut self, k: &Kid, world: &World, sights: &Sights) -> String {
        let mut count = [0u32; Mat::ALL.len()];
        let (cx, cy) = (k.x as i32, k.y as i32);
        for y in ((cy - 120).max(0)..(cy + 30).min(H as i32)).step_by(3) {
            for x in ((cx - 150).max(0)..(cx + 150).min(W as i32)).step_by(3) {
                count[world.raw(x, y) as usize] += 1;
            }
        }
        let n = |ms: &[Mat]| ms.iter().map(|m| count[*m as usize]).sum::<u32>();
        let mut topics: Vec<(Topic, f32)> = Vec::new();
        let mut add = |t: Topic, w: f32, on: bool| {
            if on {
                topics.push((t, w));
            }
        };
        add(Topic::Fire, 4.0, n(&[Mat::Fire, Mat::Ember]) > 6);
        add(Topic::Lava, 4.0, n(&[Mat::Lava, Mat::Molten, Mat::MoltenSteel]) > 10);
        add(Topic::Water, 2.0, n(&[Mat::Water]) > 200);
        add(Topic::Snow, 2.5, n(&[Mat::Snow]) > 30);
        add(Topic::Ice, 1.5, n(&[Mat::Ice]) > 20);
        add(Topic::Tree, 1.5, n(&[Mat::Leaf, Mat::Trunk, Mat::Branch]) > 30);
        add(Topic::Sand, 1.5, n(&[Mat::Sand]) > 80);
        add(Topic::Powder, 3.0, n(&[Mat::Gunpowder]) > 10);
        add(Topic::Mud, 2.0, n(&[Mat::Mud]) > 20);
        add(Topic::Smoke, 2.0, n(&[Mat::Smoke, Mat::Steam]) > 30);
        add(Topic::Cloud, 1.0, n(&[Mat::Cloud]) > 30);
        add(Topic::Glass, 1.5, n(&[Mat::Glass]) > 15);
        add(Topic::Rust, 1.5, n(&[Mat::Rust]) > 15);
        add(Topic::Oil, 2.0, n(&[Mat::Oil]) > 20);
        add(Topic::Whale, 6.0, sights.whale);
        add(Topic::Plane, 6.0, sights.plane);
        add(Topic::Truck, 6.0, sights.truck);
        add(Topic::Fireworks, 6.0, sights.fireworks);
        add(Topic::Cars, 2.5, sights.cars > 0);
        add(Topic::Blocks, 1.5, sights.blocks > 4);
        add(Topic::Night, 2.0, sights.night);
        add(Topic::Hot, 2.0, sights.temp_c > 30.0);
        add(Topic::Cold, 2.0, sights.temp_c < 2.0);
        add(Topic::Silly, 1.0, true);
        // Not the same thing again so soon.
        let fresh: Vec<_> = topics.iter().copied().filter(|(t, _)| !self.recent.contains(t)).collect();
        if !fresh.is_empty() {
            topics = fresh;
        } else {
            topics.retain(|(t, _)| self.recent.last() != Some(t));
        }
        if topics.is_empty() {
            topics.push((Topic::Silly, 1.0));
        }
        let total: f32 = topics.iter().map(|t| t.1).sum();
        let mut r = self.rand() * total;
        let mut topic = topics[0].0;
        for (t, w) in &topics {
            if r < *w {
                topic = *t;
                break;
            }
            r -= w;
        }
        self.recent.push(topic);
        if self.recent.len() > 4 {
            self.recent.remove(0);
        }
        self.pick(topic.lines()).to_string()
    }

    pub fn draw(&self, view: View, time: f32) {
        if let Some(k) = &self.kid {
            draw_kid(k, view, time);
        }
    }

    /// The speech bubble, over everything else in the picture.
    pub fn draw_bubble(&self, view: View, ui: &Ui) {
        let Some(k) = &self.kid else { return };
        let Some(b) = &k.bubble else { return };
        let fade = ((b.life + 0.3 - b.age) / 0.3).clamp(0.0, 1.0);
        if fade <= 0.0 {
            return;
        }
        let size = 16u16;
        let pad = 9.0;
        let max_w = 230.0;
        let tw = ui.text_w(&b.text, size).min(max_w);
        let th = ui.wrapped_height(&b.text, tw + 1.0, size);
        let head = view.to_screen(k.centre() - vec2(0.0, 13.0 * PIX));
        // It pops up: a little bounce as it appears.
        let pop = 1.0 - (1.0 - (b.age / 0.18).min(1.0)).powi(2);
        let (w, h) = (tw + pad * 2.0, th + pad * 2.0 - 3.0);
        let area = view.visible();
        let x = (head.x - w * 0.3).clamp(area.x + 4.0, (area.x + area.w - w - 4.0).max(area.x + 4.0));
        let y = (head.y - 34.0 - h - (1.0 - pop) * 8.0).clamp(area.y + 4.0, area.y + area.h - h - 4.0);
        let r = Rect::new(x, y, w, h);
        let a = fade * pop.max(0.2);
        let paper = Color::new(1.0, 1.0, 1.0, 0.97 * a);
        let ink = if b.ouch { Color::new(0.85, 0.12, 0.12, a) } else { Color::new(0.1, 0.1, 0.14, a) };
        // The tail points at his head.
        let base = vec2(head.x.clamp(r.x + 14.0, r.x + r.w - 26.0), r.y + r.h - 1.0);
        let tip = vec2(head.x, (head.y - 4.0).max(r.y + r.h + 4.0));
        rrect(Rect::new(r.x + 2.0, r.y + 3.0, r.w, r.h), 12.0, Color::new(0.0, 0.0, 0.0, 0.25 * a));
        rrect(r, 12.0, paper);
        rrect_lines(r, 12.0, 2.5, ink);
        let (b0, b1) = (base, base + vec2(12.0, 0.0));
        draw_triangle(b0, b1, tip, paper);
        draw_line(b0.x, b0.y, tip.x, tip.y, 2.5, ink);
        draw_line(b1.x, b1.y, tip.x, tip.y, 2.5, ink);
        draw_line(b0.x + 1.5, b0.y - 0.5, b1.x - 1.5, b1.y - 0.5, 3.5, paper);
        ui.wrapped(&b.text, r.x + pad, r.y + pad + size as f32 * 0.78, tw + 1.0, size, ink);
    }
}

// ----- remarks ------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Topic {
    Fire,
    Lava,
    Water,
    Snow,
    Ice,
    Tree,
    Sand,
    Powder,
    Mud,
    Smoke,
    Cloud,
    Glass,
    Rust,
    Oil,
    Whale,
    Plane,
    Truck,
    Fireworks,
    Cars,
    Blocks,
    Night,
    Hot,
    Cold,
    Silly,
}

impl Topic {
    fn lines(self) -> &'static [&'static str] {
        match self {
            Topic::Fire => &["Ça sent le barbecue ! Quelqu'un a des chamallows ?", "Le feu, c'est joli... mais de loin !", "Au feu ! Pin-pon ! Euh, c'est qui qui appelle les pompiers ?"],
            Topic::Lava => &["Le sol est en lave ! Vite, sur le canapé !", "De la lave ! Mes chaussettes vont fondre.", "C'est sûrement là qu'habitent les dragons."],
            Topic::Water => &["Il est où mon maillot de bain ?", "Je parie qu'il y a des requins là-dedans.", "Toute cette eau... et j'ai même pas soif."],
            Topic::Snow => &["Bataille de boules de neige !", "Brrr, j'aurais dû mettre un bonnet.", "Je vais faire un bonhomme de neige. Avec un dinosaure dessus."],
            Topic::Ice => &["Attention, ça glisse ! On dirait une patinoire.", "De la glace ! Moi je la préfère à la fraise."],
            Topic::Tree => &["Un arbre ! Parfait pour une cabane.", "Les arbres, c'est des brocolis géants.", "Si je grimpe tout en haut, je verrai ma maison ?"],
            Topic::Sand => &["Du sable ! Je vais faire un château fort.", "Le sable, ça gratte dans les chaussettes."],
            Topic::Powder => &["De la poudre noire... Je touche pas, promis !", "Ça, c'est pas du chocolat en poudre."],
            Topic::Mud => &["De la boue ! Mes chaussettes vont adorer.", "Un bain de boue, comme les cochons !"],
            Topic::Smoke => &["Ça fume ! Quelqu'un a oublié le gâteau dans le four ?", "Pouah, ça sent le brûlé par ici."],
            Topic::Cloud => &["Ce nuage ressemble à un dinosaure !", "Il va pleuvoir, je le sens dans mes chaussettes."],
            Topic::Glass => &["Du verre ! On voit à travers, c'est magique.", "Attention, le verre ça coupe."],
            Topic::Rust => &["Tout ça rouille ! Il faudrait un coup de peinture.", "C'est orange, c'est de la rouille. Je sais plein de trucs, moi."],
            Topic::Oil => &["Du pétrole ! Ça glisse et ça sent pas bon.", "Beurk, c'est tout gluant."],
            Topic::Whale => &["Une baleine ! Elle a oublié son mouchoir.", "Coucou la baleine ! Tu veux être ma copine ?"],
            Topic::Plane => &["Un avion en forme de baleine ! C'est le Béluga !", "Plus tard, je serai pilote d'avion."],
            Topic::Truck => &["Pin-pon ! Plus tard, je serai pompier.", "Allez les pompiers, vous allez y arriver !"],
            Topic::Fireworks => &["Waouh, un feu d'artifice ! C'est le 14 juillet ?", "Ooooh ! Aaaah ! Encore !"],
            Topic::Cars => &["Vroum vroum ! Je peux conduire ?", "Celle-là, elle va super vite !"],
            Topic::Blocks => &["Belle construction ! Elle va tenir... j'espère.", "On dirait un château. Il manque juste un dragon."],
            Topic::Night => &["Il fait nuit... c'est pas déjà l'heure d'aller au lit ?", "J'ai pas peur du noir. Enfin, presque pas.", "Regardez toutes ces étoiles !"],
            Topic::Hot => &["Il fait chaud ! Je veux une glace à la fraise.", "Même mon dinosaure transpire."],
            Topic::Cold => &["Il fait tellement froid que mon dino grelotte.", "Brrr ! Vivement l'été."],
            Topic::Silly => &["Rawr ! Je suis un T-rex !", "J'ai faim. C'est quand, le goûter ?", "C'est moi le roi du monde !", "Tiens, un caillou. Il est trop beau.", "Je compte jusqu'à mille : un, deux, trois... bon, plus tard."],
        }
    }
}

const HELLO: &[&str] = &["Coucou ! Je viens me promener.", "Salut ! Je viens voir ce que tu construis.", "Me revoilà !"];
const BYE: &[&str] = &["Bon, j'ai faim, je rentre goûter. Salut !", "À plus tard !", "Salut ! Je reviendrai !"];
const NO_SWIM: &[&str] = &["Pas de baignade sans maillot !", "Oh non, j'ai pas mes brassards."];
const WHEELED: &[&str] = &["Je suis le roi de la roue !", "Ça tourne, ça tourne..."];
const FLIPPED: &[&str] = &["Tadaa !", "Même pas peur !", "Dix sur dix !"];
const RAIN: &[&str] = &["Il pleut ! Je suis tout mouillé.", "Il pleut des cordes... et des escargots ?"];

// ----- the sprite ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Arms {
    Down,
    Up,
    Star,
    Wave,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Legs {
    Stand,
    StepA,
    StepB,
    Apart,
    Tuck,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Face {
    Grin,
    Talk,
    Ouch,
}

const HAIR: [&str; 4] = ["...hhhhhh...", "..hhhhhhhh..", ".hhhhhhhhhh.", ".hshhsshhsh."];
const GRIN: [&str; 4] = [".sPssssssPs.", ".ssmwwwwmss.", "..ssmmmmss..", "...ssssss..."];
const TALK: [&str; 4] = [".sPssssssPs.", ".sssmmmmsss.", "..sssmmsss..", "...ssssss..."];
const OUCH_EYES: &str = "ssseesseesss";
/// The hoodie, with the dinosaur on the front (the arms are drawn apart).
const TORSO: [&str; 8] = ["..yyyyyyyy..", "..yyyyyyyy..", "..yyyygkyy..", "..yyyggggy..", "..yyggwwyy..", "..yygggggy..", "..yyygygyy..", "..YYYYYYYY.."];
const LEGS_STAND: [&str; 5] = ["....bbbbbbbb....", "....bbb..bbb....", "....bbb..bbb....", "....BBB..BBB....", "...rkkk..kkkr..."];
const LEGS_STEP: [&str; 5] = ["....bbbbbbbb....", "...bbb...bbb....", "...bbb...bbb....", "..BBB....BBB....", ".rkkk....kkkr..."];
const LEGS_APART: [&str; 5] = ["....bbbbbbbb....", "...bbbb..bbbb...", "..bbb......bbb..", ".BBB........BBB.", "rkk..........kkr"];
const LEGS_TUCK: [&str; 5] = ["...bbbbbbbbbb...", "...BBBb..bBBB...", "...rkkk..kkkr...", "................", "................"];

pub type Sprite = [[u8; SW]; SH];

fn put(g: &mut Sprite, rows: &[&str], top: usize, left: usize) {
    for (j, r) in rows.iter().enumerate() {
        for (i, c) in r.bytes().enumerate() {
            if c != b'.' {
                g[top + j][left + i] = c;
            }
        }
    }
}

/// The left arm as (column, row, colour); the right one is its mirror image.
fn arm(a: Arms, right: bool) -> Vec<(usize, usize, u8)> {
    let a = match (a, right) {
        (Arms::Wave, true) => Arms::Up,
        (Arms::Wave, false) => Arms::Down,
        (a, _) => a,
    };
    let mut px = match a {
        Arms::Down => {
            let mut v: Vec<_> = (12..=17).map(|r| (3, r, b'Y')).collect();
            v.push((3, 18, b's'));
            v
        }
        Arms::Up => {
            let mut v: Vec<_> = (4..=11).map(|r| (2, r, b'Y')).collect();
            v.push((3, 12, b'Y'));
            v.push((2, 3, b's'));
            v
        }
        _ => vec![(3, 12, b'Y'), (2, 11, b'Y'), (2, 10, b'Y'), (1, 9, b'Y'), (1, 8, b'Y'), (0, 7, b's')],
    };
    if right {
        for p in &mut px {
            p.0 = SW - 1 - p.0;
        }
    }
    px
}

/// His picture, as colour letters (see `colour`), '.' where there is nothing. `look` moves his
/// eyes towards where he is going (-1, 0, 1).
pub fn sprite(arms: Arms, legs: Legs, face: Face, look: i32) -> Sprite {
    let mut g = [[b'.'; SW]; SH];
    put(&mut g, &HAIR, 2, 2);
    let eyes = if face == Face::Ouch {
        OUCH_EYES.to_string()
    } else {
        (0..12).map(|i| if i == 3 + look || i == 8 + look { 'e' } else { 's' }).collect()
    };
    put(&mut g, &[eyes.as_str()], 6, 2);
    let mouth = match face {
        Face::Grin => GRIN,
        _ => TALK,
    };
    put(&mut g, &mouth, 7, 2);
    put(&mut g, &TORSO, 11, 2);
    let legs: Vec<String> = match legs {
        Legs::Stand => LEGS_STAND.iter().map(|s| s.to_string()).collect(),
        Legs::StepA => LEGS_STEP.iter().map(|s| s.to_string()).collect(),
        Legs::StepB => LEGS_STEP.iter().map(|s| s.chars().rev().collect()).collect(),
        Legs::Apart => LEGS_APART.iter().map(|s| s.to_string()).collect(),
        Legs::Tuck => LEGS_TUCK.iter().map(|s| s.to_string()).collect(),
    };
    let legs: Vec<&str> = legs.iter().map(|s| s.as_str()).collect();
    put(&mut g, &legs, 19, 0);
    for right in [false, true] {
        for (i, j, c) in arm(arms, right) {
            g[j][i] = c;
        }
    }
    g
}

fn colour(c: u8) -> Color {
    let (r, g, b) = match c {
        b'h' => (240, 210, 122),
        b's' => (248, 202, 168),
        b'P' => (240, 152, 140),
        b'e' => (38, 66, 118),
        b'm' => (128, 40, 44),
        b'w' => (255, 255, 255),
        b'y' => (234, 198, 74),
        b'Y' => (204, 162, 48),
        b'g' => (96, 128, 70),
        b'b' => (94, 170, 226),
        b'B' => (66, 132, 196),
        b'k' => (48, 58, 88),
        b'r' => (222, 104, 122),
        _ => (34, 28, 40),
    };
    Color::from_rgba(r, g, b, 255)
}

/// The pose for what he is doing.
pub fn pose(k: &Kid, time: f32) -> (Arms, Legs, Face) {
    let talking = k.talking();
    let face = if k.act == Act::Hurt || k.bubble.as_ref().map_or(false, |b| b.ouch && b.age < 1.0) {
        Face::Ouch
    } else if talking && (time * 7.0).sin() > 0.0 {
        Face::Talk
    } else {
        Face::Grin
    };
    let step = match (k.stride / 3.5) as u32 % 4 {
        0 => Legs::StepA,
        2 => Legs::StepB,
        _ => Legs::Stand,
    };
    match k.act {
        Act::Hurt => (Arms::Up, Legs::Apart, face),
        Act::Cartwheel => (Arms::Star, Legs::Apart, face),
        Act::Flip => (Arms::Down, Legs::Tuck, face),
        Act::Leap => (Arms::Star, Legs::Apart, face),
        _ if k.air => (Arms::Up, Legs::Stand, face),
        Act::Idle => (Arms::Wave, Legs::Stand, face),
        _ => (Arms::Down, step, face),
    }
}

fn draw_kid(k: &Kid, view: View, time: f32) {
    let (arms, legs, face) = pose(k, time);
    let look = if k.act == Act::Idle { 0 } else { k.dir as i32 };
    let g = sprite(arms, legs, face, look);
    let centre = k.centre();
    // A little bob as he walks.
    let bob = if k.act == Act::Walk && !k.air { -((k.stride / 3.5 * std::f32::consts::PI).sin().abs()) * 0.6 } else { 0.0 };
    let (sa, ca) = k.angle.sin_cos();
    let size = PIX * view.s;
    let at = |i: i32, j: i32| -> Vec2 {
        let (lx, ly) = ((i as f32 + 0.5 - SW as f32 / 2.0) * PIX, (j as f32 + 0.5 - SH as f32 / 2.0) * PIX + bob);
        view.to_screen(centre + vec2(lx * ca - ly * sa, lx * sa + ly * ca))
    };
    let filled = |i: i32, j: i32| i >= 0 && j >= 0 && (i as usize) < SW && (j as usize) < SH && g[j as usize][i as usize] != b'.';
    let pixel = |p: Vec2, col: Color| {
        let side = size + 0.6;
        if k.angle == 0.0 {
            draw_rectangle(p.x - side / 2.0, p.y - side / 2.0, side, side, col);
        } else {
            draw_rectangle_ex(p.x, p.y, side, side, DrawRectangleParams { offset: vec2(0.5, 0.5), rotation: k.angle, color: col });
        }
    };
    // A dark outline round him, so he stands out on any background and in every view.
    let ink = colour(b'#');
    for j in -1..=SH as i32 {
        for i in -1..=SW as i32 {
            if !filled(i, j) && [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|(dx, dy)| filled(i + dx, j + dy)) {
                pixel(at(i, j), ink);
            }
        }
    }
    for j in 0..SH as i32 {
        for i in 0..SW as i32 {
            if filled(i, j) {
                pixel(at(i, j), colour(g[j as usize][i as usize]));
            }
        }
    }
    // Stars round his head when he is hurt.
    if k.act == Act::Hurt {
        let head = centre - vec2(0.0, 13.0 * PIX);
        for n in 0..3 {
            let a = time * 6.0 + n as f32 * TAU / 3.0;
            let p = view.to_screen(head + vec2(a.cos() * 8.0, a.sin() * 2.5 - 2.0));
            draw_poly(p.x, p.y, 5, (1.6 * view.s).max(2.5), a.to_degrees(), Color::from_rgba(255, 220, 60, 255));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat stone floor across the world at row `ground`.
    fn flat(ground: i32) -> World {
        let mut w = World::new_static();
        for x in 0..W as i32 {
            for y in ground..H as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        w
    }

    fn run(kids: &mut Kids, world: &World, secs: f32) {
        for _ in 0..(secs * 60.0) as u32 {
            kids.update(1.0 / 60.0, world, &Sights::default());
        }
    }

    #[test]
    fn every_pose_is_a_whole_picture() {
        for arms in [Arms::Down, Arms::Up, Arms::Star, Arms::Wave] {
            for legs in [Legs::Stand, Legs::StepA, Legs::StepB, Legs::Apart, Legs::Tuck] {
                for face in [Face::Grin, Face::Talk, Face::Ouch] {
                    let g = sprite(arms, legs, face, 1);
                    let n = g.iter().flatten().filter(|&&c| c != b'.').count();
                    assert!(n > 150, "{arms:?} {legs:?} {face:?}: {n}");
                    // Every letter has its colour (the outline colour is only for the outline).
                    for &c in g.iter().flatten().filter(|&&c| c != b'.') {
                        assert_ne!(colour(c), colour(b'#'), "{}", c as char);
                    }
                }
            }
        }
        // The yellow hoodie, the green dinosaur, the blue joggers and the dark socks are there.
        let g = sprite(Arms::Down, Legs::Stand, Face::Grin, 0);
        for c in [b'h', b'y', b'g', b'b', b'k', b'w'] {
            assert!(g.iter().flatten().any(|&x| x == c), "{}", c as char);
        }
    }

    #[test]
    fn he_comes_now_and_then_and_walks_on_the_ground() {
        let world = flat(400);
        let mut kids = Kids::new();
        run(&mut kids, &world, 20.0);
        assert!(kids.kid.is_none(), "not straight away");
        let mut seen = false;
        let mut xs = Vec::new();
        for _ in 0..60 * 60 {
            kids.update(1.0 / 60.0, &world, &Sights::default());
            if let Some(k) = &kids.kid {
                seen = true;
                xs.push(k.x);
                if !k.air && k.act != Act::Leap {
                    assert!((k.y - 400.0).abs() < 1.5, "on the ground: {}", k.y);
                }
            }
        }
        assert!(seen, "he came within a minute and a half");
        let (lo, hi) = xs.iter().fold((f32::MAX, f32::MIN), |a, &x| (a.0.min(x), a.1.max(x)));
        assert!(hi - lo > 150.0, "he walked about: {lo}..{hi}");
    }

    #[test]
    fn he_jumps_a_low_wall_and_turns_back_at_a_high_one() {
        let mut world = flat(400);
        for y in 388..400 {
            for x in 300..306 {
                world.set(x, y, Mat::Stone);
            }
        }
        for y in 300..400 {
            for x in 600..606 {
                world.set(x, y, Mat::Stone);
            }
        }
        let mut kids = Kids::new();
        assert!(kids.summon(&world));
        let k = kids.kid.as_mut().unwrap();
        k.x = 250.0;
        k.y = 400.0;
        k.dir = 1.0;
        k.trick_in = 1000.0;
        k.stay = 1000.0;
        let mut max_x: f32 = 0.0;
        for _ in 0..60 * 40 {
            kids.update(1.0 / 60.0, &world, &Sights::default());
            let k = kids.kid.as_ref().expect("still here");
            max_x = max_x.max(k.x);
        }
        assert!(max_x > 310.0, "over the low wall: {max_x}");
        assert!(max_x < 600.0, "not through the high wall: {max_x}");
    }

    #[test]
    fn he_turns_cartwheels_and_somersaults() {
        let world = flat(400);
        let mut kids = Kids::new();
        assert!(kids.summon(&world));
        kids.kid.as_mut().unwrap().stay = 1000.0;
        let (mut wheel, mut flip, mut spun) = (false, false, 0.0f32);
        for _ in 0..60 * 120 {
            kids.update(1.0 / 60.0, &world, &Sights::default());
            let k = kids.kid.as_ref().unwrap();
            wheel |= k.act == Act::Cartwheel;
            flip |= k.act == Act::Flip;
            spun = spun.max(k.angle.abs());
        }
        assert!(wheel && flip, "cartwheel {wheel}, flip {flip}");
        assert!(spun > 5.0, "he went right round: {spun}");
    }

    #[test]
    fn nothing_hurts_him_but_he_says_so() {
        let mut world = flat(400);
        let mut kids = Kids::new();
        assert!(kids.summon(&world));
        let k = kids.kid.as_mut().unwrap();
        k.x = 480.0;
        k.y = 400.0;
        k.bubble = None;
        for y in 380..400 {
            for x in 470..490 {
                world.set(x, y, Mat::Lava);
            }
        }
        kids.update(1.0 / 60.0, &world, &Sights::default());
        let k = kids.kid.as_ref().expect("lava does not get rid of him");
        let b = k.bubble.as_ref().expect("he shouts");
        assert!(b.ouch && (b.text.contains("Aïe") || b.text.contains("Ouille")), "{}", b.text);
        // An explosion only makes him cross.
        kids.kid.as_mut().unwrap().ouch_in = 0.0;
        kids.ouch(Hit::Blast);
        assert!(kids.kid.is_some());
        let b = kids.kid.as_ref().unwrap().bubble.clone().unwrap();
        assert!(b.ouch && b.text != "", "{}", b.text);
    }

    #[test]
    fn clicks_make_him_cross_then_he_goes_home() {
        let world = flat(400);
        let mut kids = Kids::new();
        assert!(kids.summon(&world));
        let k = kids.kid.as_mut().unwrap();
        k.x = 480.0;
        k.y = 400.0;
        assert!(k.contains(vec2(480.0, 390.0)), "his middle");
        assert!(!k.contains(vec2(480.0, 360.0)), "above his head");
        kids.ouch(Hit::Click);
        kids.ouch(Hit::Click);
        assert_eq!(kids.kid.as_ref().unwrap().bubble.as_ref().unwrap().text, "Hé ho, ça va pas non ?!");
        kids.ouch(Hit::Click);
        kids.ouch(Hit::Click);
        run(&mut kids, &world, 8.0);
        assert!(kids.kid.is_none(), "he left");
    }

    #[test]
    fn he_talks_about_what_he_sees() {
        let mut world = flat(400);
        for y in 380..400 {
            for x in 440..520 {
                world.set(x, y, Mat::Snow);
            }
        }
        let mut kids = Kids::new();
        assert!(kids.summon(&world));
        let k = kids.kid.clone().unwrap();
        let k = Kid { x: 480.0, y: 380.0, ..k };
        let sights = Sights { whale: true, ..Default::default() };
        let mut said = Vec::new();
        for _ in 0..6 {
            said.push(kids.remark(&k, &world, &sights));
        }
        assert!(said.iter().any(|s| Topic::Whale.lines().contains(&s.as_str())), "{said:?}");
        assert!(said.iter().any(|s| Topic::Snow.lines().contains(&s.as_str())), "{said:?}");
        // Not the same topic twice in a row.
        for w in said.windows(2) {
            assert_ne!(w[0], w[1]);
        }
    }

    #[test]
    fn he_leaves_after_his_walk() {
        let world = flat(400);
        let mut kids = Kids::new();
        assert!(kids.summon(&world));
        run(&mut kids, &world, VISIT.1 + 60.0);
        assert!(kids.kid.is_none(), "he went home");
    }
}
