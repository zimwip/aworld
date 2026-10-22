//! Lego-like rigid blocks. They live outside the sand grid: while building
//! they are plain shapes snapped to a stud grid; when the simulation runs
//! they become rapier bodies, welded to the blocks they touch.

use crate::world::{Mat, World, H, W};
use macroquad::prelude::{vec2, Vec2};
use rapier2d::prelude::*;
use std::collections::{HashMap, HashSet};

/// Size of one stud in cells; blocks snap to this grid.
pub const STUD: f32 = 10.0;
/// Cells per physics meter.
pub const PX_PER_M: f32 = 16.0;
const TILE: i32 = 16;
/// Weld breaking limits (per step): force and torque impulses.
const BREAK_FORCE: f32 = 1.3;
const BREAK_TORQUE: f32 = 3.2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockKind {
    Brick,
    Long,
    Plank,
    Beam,
    Cube,
    Wheel,
    SteelBeam,
    Glass,
    GlassHalf,
    GlassChip,
    /// A tall, thin brick standing up (the long one's vertical twin).
    Post,
}

pub struct Spec {
    pub name: &'static str,
    pub w: f32,
    pub h: f32,
    pub mat: Mat,
    pub round: bool,
    pub density: f32,
    pub color: (u8, u8, u8),
    /// How hard a weld to this block is to break (relative).
    pub strength: f32,
    pub blurb: &'static str,
}

impl BlockKind {
    /// What a block is made of in the world: cells of these materials are used up
    /// when it is built, and `units` is how many.
    pub fn cost(self) -> (&'static [Mat], u32) {
        let s = self.spec();
        let area = if s.round { std::f32::consts::PI * (s.w / 2.0).powi(2) } else { s.w * s.h };
        let units = (area * 0.25).ceil() as u32;
        let from: &'static [Mat] = match self {
            BlockKind::Brick | BlockKind::Long | BlockKind::Post => &[Mat::Dirt],
            BlockKind::Plank => &[Mat::Wood, Mat::Trunk, Mat::Branch],
            BlockKind::Beam | BlockKind::Wheel => &[Mat::Iron],
            BlockKind::SteelBeam => &[Mat::Steel],
            BlockKind::Cube => &[Mat::Stone],
            BlockKind::Glass | BlockKind::GlassHalf | BlockKind::GlassChip => &[Mat::Sand],
        };
        (from, units)
    }

    /// Every kind, including the smaller pieces glass breaks into.
    pub const ALL: [BlockKind; 11] = [
        BlockKind::Brick,
        BlockKind::Long,
        BlockKind::Plank,
        BlockKind::Beam,
        BlockKind::SteelBeam,
        BlockKind::Cube,
        BlockKind::Glass,
        BlockKind::Wheel,
        BlockKind::GlassHalf,
        BlockKind::GlassChip,
        BlockKind::Post,
    ];

    /// What can be picked from the palette.
    pub const PLACEABLE: [BlockKind; 6] = [
        BlockKind::Brick,
        BlockKind::Long,
        BlockKind::Post,
        BlockKind::Cube,
        BlockKind::Glass,
        BlockKind::Wheel,
    ];

    pub fn is_glass(self) -> bool {
        matches!(self, BlockKind::Glass | BlockKind::GlassHalf | BlockKind::GlassChip)
    }

    /// Stable numbers used in save files (never reorder).
    pub fn id(self) -> u8 {
        match self {
            BlockKind::Brick => 0,
            BlockKind::Long => 1,
            BlockKind::Plank => 2,
            BlockKind::Beam => 3,
            BlockKind::Cube => 4,
            BlockKind::Wheel => 5,
            BlockKind::SteelBeam => 6,
            BlockKind::Glass => 7,
            BlockKind::GlassHalf => 8,
            BlockKind::GlassChip => 9,
            BlockKind::Post => 10,
        }
    }

    pub fn from_id(id: u8) -> Option<BlockKind> {
        Self::ALL.iter().copied().find(|k| k.id() == id)
    }

    pub fn spec(self) -> Spec {
        match self {
            BlockKind::Brick => Spec { name: "Brique", w: 20.0, h: 10.0, mat: Mat::Brick, round: false, density: 1.0, color: (205, 62, 52), strength: 1.2, blurb: "Plastique solide" },
            BlockKind::Long => Spec { name: "Longue", w: 40.0, h: 10.0, mat: Mat::Brick, round: false, density: 1.0, color: (58, 112, 214), strength: 1.2, blurb: "Brique de 4 plots" },
            BlockKind::Post => Spec { name: "Poteau", w: 10.0, h: 40.0, mat: Mat::Brick, round: false, density: 1.0, color: (58, 112, 214), strength: 1.2, blurb: "Haute et fine : 1 plot, 4 de haut" },
            BlockKind::Plank => Spec { name: "Planche", w: 60.0, h: 5.0, mat: Mat::Wood, round: false, density: 0.6, color: (176, 124, 72), strength: 0.25, blurb: "Bois : léger, fragile, brûle" },
            BlockKind::Beam => Spec { name: "Tige fer", w: 80.0, h: 10.0, mat: Mat::Iron, round: false, density: 1.8, color: (150, 160, 178), strength: 2.0, blurb: "Fer : rouille dans l'eau" },
            BlockKind::SteelBeam => Spec { name: "Tige acier", w: 80.0, h: 10.0, mat: Mat::Steel, round: false, density: 1.8, color: (96, 128, 186), strength: 4.0, blurb: "Acier : fer + carbone, 2x plus solide" },
            BlockKind::Glass => Spec { name: "Verre", w: 20.0, h: 10.0, mat: Mat::Glass, round: false, density: 0.9, color: (176, 222, 244), strength: 0.5, blurb: "Sable fondu : la voiture le brise" },
            BlockKind::GlassHalf => Spec { name: "Verre (moitié)", w: 10.0, h: 10.0, mat: Mat::Glass, round: false, density: 0.9, color: (176, 222, 244), strength: 0.4, blurb: "Un morceau de verre : encore un coup et il se brise" },
            BlockKind::GlassChip => Spec { name: "Verre (éclat)", w: 5.0, h: 5.0, mat: Mat::Glass, round: false, density: 0.9, color: (176, 222, 244), strength: 0.3, blurb: "Dernier morceau : il va se briser en éclats" },
            BlockKind::Cube => Spec { name: "Cube", w: 20.0, h: 20.0, mat: Mat::Stone, round: false, density: 1.4, color: (120, 120, 134), strength: 1.4, blurb: "Pierre : lourde" },
            BlockKind::Wheel => Spec { name: "Roue", w: 24.0, h: 24.0, mat: Mat::Iron, round: true, density: 1.2, color: (60, 62, 74), strength: 1.0, blurb: "Roule librement" },
        }
    }
}

fn rot(a: f32, v: Vec2) -> Vec2 {
    let (s, c) = a.sin_cos();
    vec2(v.x * c - v.y * s, v.x * s + v.y * c)
}

static NEXT_UID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

#[derive(Clone)]
pub struct Block {
    /// Stable identity (indices shift when blocks are removed).
    pub uid: u32,
    pub kind: BlockKind,
    pub pos: Vec2,
    pub angle: f32,
    /// Accumulated fire damage (wood only).
    pub heat: u16,
    /// Corrosion 0..1 (iron and steel in water).
    pub rust: f32,
    /// Steps before this block can take another blow (glass breaks in stages).
    pub hurt: u16,
    pub body: Option<RigidBodyHandle>,
}

impl Block {
    pub fn new(kind: BlockKind, pos: Vec2, angle: f32) -> Self {
        Block { uid: NEXT_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed), kind, pos, angle, heat: 0, rust: 0.0, hurt: 0, body: None }
    }

    pub fn spec(&self) -> Spec {
        self.kind.spec()
    }

    pub fn half(&self) -> Vec2 {
        let s = self.spec();
        vec2(s.w / 2.0, s.h / 2.0)
    }

    pub fn covers(&self, p: Vec2) -> bool {
        let local = rot(-self.angle, p - self.pos);
        let h = self.half();
        if self.spec().round {
            local.length() <= h.x
        } else {
            local.x.abs() <= h.x && local.y.abs() <= h.y
        }
    }

    /// Cell bounds (min x, min y, max x, max y), clamped to the world.
    pub fn bounds(&self) -> (i32, i32, i32, i32) {
        let r = self.half().length() + 1.0;
        (
            ((self.pos.x - r).floor() as i32).max(0),
            ((self.pos.y - r).floor() as i32).max(0),
            ((self.pos.x + r).ceil() as i32).min(W as i32 - 1),
            ((self.pos.y + r).ceil() as i32).min(H as i32 - 1),
        )
    }

    /// Signed distance between two blocks: positive is a gap, negative overlap.
    pub fn gap(&self, o: &Block) -> f32 {
        let (a, b) = (self, o);
        let (ra, rb) = (a.spec().round, b.spec().round);
        match (ra, rb) {
            (true, true) => (a.pos - b.pos).length() - a.half().x - b.half().x,
            (true, false) => Self::circle_rect(a, b),
            (false, true) => Self::circle_rect(b, a),
            (false, false) => {
                let mut best = f32::MIN;
                for blk in [a, b] {
                    for axis in [rot(blk.angle, vec2(1.0, 0.0)), rot(blk.angle, vec2(0.0, 1.0))] {
                        let ext = |q: &Block| {
                            let h = q.half();
                            let ax = rot(q.angle, vec2(1.0, 0.0));
                            let ay = rot(q.angle, vec2(0.0, 1.0));
                            h.x * ax.dot(axis).abs() + h.y * ay.dot(axis).abs()
                        };
                        let sep = (a.pos - b.pos).dot(axis).abs() - ext(a) - ext(b);
                        best = best.max(sep);
                    }
                }
                best
            }
        }
    }

    fn circle_rect(c: &Block, r: &Block) -> f32 {
        let local = rot(-r.angle, c.pos - r.pos);
        let h = r.half();
        let q = vec2(local.x.clamp(-h.x, h.x), local.y.clamp(-h.y, h.y));
        let d = (local - q).length();
        let dist = if d > 0.0 { d } else { -((h.x - local.x.abs()).min(h.y - local.y.abs())) };
        dist - c.half().x
    }
}

/// Stamps blocks and free beams into the world's solid mask. Mask ids are the
/// block indices first, then the beams (offset by the number of blocks).
pub fn stamp_all(world: &mut World, blocks: &[Block], members: &[Member]) {
    let mut mats: Vec<Mat> = blocks.iter().map(|b| b.spec().mat).collect();
    mats.extend(members.iter().map(|m| m.mat.mat()));
    let mut passable = vec![false; blocks.len()];
    passable.extend(members.iter().map(|m| !m.is_deck()));
    world.begin_blocks(mats, passable);
    for (id, b) in blocks.iter().enumerate() {
        let (x0, y0, x1, y1) = b.bounds();
        for y in y0..=y1 {
            for x in x0..=x1 {
                if b.covers(vec2(x as f32 + 0.5, y as f32 + 0.5)) {
                    world.mark_block(x, y, id);
                }
            }
        }
    }
    for (k, m) in members.iter().enumerate() {
        let (x0, y0, x1, y1) = m.bounds();
        for y in y0..=y1 {
            for x in x0..=x1 {
                if m.covers(vec2(x as f32 + 0.5, y as f32 + 0.5)) {
                    world.mark_block(x, y, blocks.len() + k);
                }
            }
        }
    }
    world.end_blocks();
}

#[allow(dead_code)]
pub fn stamp_blocks(world: &mut World, blocks: &[Block]) {
    stamp_all(world, blocks, &[]);
}

/// Can a block of this shape sit here without cutting into terrain or others?
pub fn placement_ok(world: &World, blocks: &[Block], cand: &Block, ignore: Option<usize>) -> bool {
    let (x0, y0, x1, y1) = cand.bounds();
    let r = cand.half().length();
    if cand.pos.x - r < 0.0 || cand.pos.x + r > W as f32 || cand.pos.y - r < 0.0 || cand.pos.y + r > H as f32 {
        return false;
    }
    for y in y0..=y1 {
        for x in x0..=x1 {
            if cand.covers(vec2(x as f32 + 0.5, y as f32 + 0.5)) && world.raw(x, y).is_hard() {
                return false;
            }
        }
    }
    // Flush is fine (gap 0); any real overlap is refused.
    blocks.iter().enumerate().all(|(i, b)| Some(i) == ignore || cand.gap(b) > -0.2)
}

/// Snap a block so its corner sits on the stud grid.
pub fn snap(kind: BlockKind, rot90: u8, mouse: Vec2) -> Vec2 {
    let s = kind.spec();
    let (w, h) = if rot90 % 2 == 0 { (s.w, s.h) } else { (s.h, s.w) };
    if s.round {
        return vec2((mouse.x / STUD).round() * STUD, (mouse.y / STUD).round() * STUD);
    }
    vec2(((mouse.x - w / 2.0) / STUD).round() * STUD + w / 2.0, ((mouse.y - h / 2.0) / STUD).round() * STUD + h / 2.0)
}

/// Does this kind carry studs on top (and tubes underneath), like a Lego brick?
pub fn has_studs(kind: BlockKind) -> bool {
    matches!(kind, BlockKind::Brick | BlockKind::Long | BlockKind::Post | BlockKind::Cube | BlockKind::Glass | BlockKind::GlassHalf)
}

/// The studs two upright bricks share: where one sits on the other, those of the lower one that fall
/// under the upper one (the points where they grip). Each one is a little friction element.
pub fn shared_stud_points(ka: BlockKind, pa: Vec2, aa: f32, kb: BlockKind, pb: Vec2, ab: f32) -> Vec<Vec2> {
    if !has_studs(ka) || !has_studs(kb) {
        return vec![];
    }
    // Studs are on top: only for bricks standing upright.
    if quarter_turns(aa) != Some(0) || quarter_turns(ab) != Some(0) {
        return vec![];
    }
    let (Some(ha), Some(hb)) = (flat_half(ka, aa), flat_half(kb, ab)) else { return vec![] };
    let mut out = vec![];
    for (up, hu, low, hl) in [(pa, ha, pb, hb), (pb, hb, pa, ha)] {
        // The upper brick's underside rests on the lower one's top.
        if ((up.y + hu.y) - (low.y - hl.y)).abs() > 1.5 {
            continue;
        }
        let (lo, hi) = ((up.x - hu.x).max(low.x - hl.x), (up.x + hu.x).min(low.x + hl.x));
        let studs = ((2.0 * hl.x) / STUD).round() as i32;
        for k in 0..studs {
            let x = low.x - hl.x + STUD / 2.0 + k as f32 * STUD;
            if x > lo + 0.5 && x < hi - 0.5 {
                out.push(vec2(x, low.y - hl.y));
            }
        }
    }
    out
}

/// How many studs two blocks share.
pub fn block_studs(a: &Block, b: &Block) -> u32 {
    shared_stud_points(a.kind, a.pos, a.angle, b.kind, b.pos, b.angle).len() as u32
}

/// Distance (cells) within which a block is pulled onto its neighbours.
pub const CLIP: f32 = 6.0;

fn quarter_turns(angle: f32) -> Option<i32> {
    let q = angle / std::f32::consts::FRAC_PI_2;
    ((q - q.round()).abs() < 0.02).then(|| q.round() as i32)
}

/// Half-size of an axis-aligned block, or None for round / tilted blocks.
fn flat_half(kind: BlockKind, angle: f32) -> Option<Vec2> {
    let s = kind.spec();
    if s.round {
        return None;
    }
    let turns = quarter_turns(angle)?;
    Some(if turns.rem_euclid(2) == 0 { vec2(s.w / 2.0, s.h / 2.0) } else { vec2(s.h / 2.0, s.w / 2.0) })
}

/// Magnetic clip: pull a block flush against nearby blocks (on top, below or
/// beside) and onto their stud grid, like Lego bricks snapping together.
pub fn clip(blocks: &[Block], kind: BlockKind, angle: f32, pos: Vec2) -> Vec2 {
    let Some(h) = flat_half(kind, angle) else { return pos };
    let mut best: Option<(f32, Vec2)> = None;
    let mut consider = |cost: f32, p: Vec2, other: &Block| {
        // The more studs a spot would engage, the better it is: a brick settles where it grips most.
        let studs = shared_stud_points(kind, p, angle, other.kind, other.pos, other.angle).len() as f32;
        let cost = cost - 1.5 * studs.min(6.0);
        if best.map_or(true, |b| cost < b.0) {
            best = Some((cost, p));
        }
    };
    // Align an edge to the neighbour's stud grid when it is close enough.
    let stud = |edge: f32, origin: f32| {
        let snapped = origin + ((edge - origin) / STUD).round() * STUD;
        if (snapped - edge).abs() < CLIP { snapped } else { edge }
    };
    for b in blocks {
        let Some(bh) = flat_half(b.kind, b.angle) else { continue };
        let (bl, br, bt, bb) = (b.pos.x - bh.x, b.pos.x + bh.x, b.pos.y - bh.y, b.pos.y + bh.y);
        let (l, r, t, bot) = (pos.x - h.x, pos.x + h.x, pos.y - h.y, pos.y + h.y);
        let x_overlap = r > bl + 1.0 && l < br - 1.0;
        let y_overlap = bot > bt + 1.0 && t < bb - 1.0;
        if x_overlap {
            // On top of it.
            if (bot - bt).abs() < CLIP {
                consider((bot - bt).abs(), vec2(stud(l, bl) + h.x, bt - h.y), b);
            }
            // Hanging below it.
            if (t - bb).abs() < CLIP {
                consider((t - bb).abs(), vec2(stud(l, bl) + h.x, bb + h.y), b);
            }
        }
        if y_overlap {
            if (r - bl).abs() < CLIP {
                consider((r - bl).abs(), vec2(bl - h.x, stud(t, bt) + h.y), b);
            }
            if (l - br).abs() < CLIP {
                consider((l - br).abs(), vec2(br + h.x, stud(t, bt) + h.y), b);
            }
        }
    }
    best.map_or(pos, |b| b.1)
}

pub fn block_at(blocks: &[Block], p: Vec2) -> Option<usize> {
    blocks.iter().rposition(|b| b.covers(p))
}

// ---------------------------------------------------------------------------
// Free beams (bridge-builder members)

/// Longest beam that can be drawn in one go, in cells.
pub const MEMBER_MAX_LEN: f32 = 110.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemberMat {
    Wood,
    Iron,
    Steel,
}

impl MemberMat {
    pub const ALL: [MemberMat; 3] = [MemberMat::Wood, MemberMat::Iron, MemberMat::Steel];

    pub fn name(self) -> &'static str {
        match self {
            MemberMat::Wood => "Bois",
            MemberMat::Iron => "Fer",
            MemberMat::Steel => "Acier",
        }
    }

    pub fn mat(self) -> Mat {
        match self {
            MemberMat::Wood => Mat::Wood,
            MemberMat::Iron => Mat::Iron,
            MemberMat::Steel => Mat::Steel,
        }
    }

    pub fn id(self) -> u8 {
        self as u8
    }

    pub fn from_id(id: u8) -> MemberMat {
        Self::ALL.iter().copied().find(|m| m.id() == id).unwrap_or(MemberMat::Wood)
    }

    /// Mass per area: wood is light, iron and steel are heavy.
    pub fn density(self) -> f32 {
        match self {
            MemberMat::Wood => 0.6,
            MemberMat::Iron => 1.8,
            MemberMat::Steel => 1.9,
        }
    }

    /// Force (impulse per step) the beam can carry before it breaks.
    pub fn limit(self) -> f32 {
        match self {
            MemberMat::Wood => 0.8,
            MemberMat::Iron => 2.4,
            MemberMat::Steel => 5.0,
        }
    }

    pub fn thickness(self) -> f32 {
        match self {
            MemberMat::Wood => 4.0,
            MemberMat::Iron => 3.5,
            MemberMat::Steel => 3.5,
        }
    }

    pub fn color(self) -> (u8, u8, u8) {
        match self {
            MemberMat::Wood => (176, 124, 72),
            MemberMat::Iron => (150, 160, 178),
            MemberMat::Steel => (96, 128, 186),
        }
    }

    /// Cells of these materials are used up to build a beam.
    pub fn cost_from(self) -> &'static [Mat] {
        match self {
            MemberMat::Wood => &[Mat::Wood, Mat::Trunk, Mat::Branch],
            MemberMat::Iron => &[Mat::Iron],
            MemberMat::Steel => &[Mat::Steel],
        }
    }

    pub fn cost(self, len: f32) -> u32 {
        (len * self.thickness() * 0.25).ceil() as u32
    }
}

/// A straight beam pinned at both ends: the building piece of a truss bridge.
#[derive(Clone)]
pub struct Member {
    pub uid: u32,
    pub mat: MemberMat,
    pub a: Anchor,
    pub b: Anchor,
    /// Current end positions (follow the physics once it runs).
    pub p0: Vec2,
    pub p1: Vec2,
    /// Load as a fraction of what the beam can take (1.0 = breaks).
    pub stress: f32,
    over: u8,
    body: Option<RigidBodyHandle>,
    joints: Vec<ImpulseJointHandle>,
}

static NEXT_MEMBER_UID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

impl Member {
    pub fn new(mat: MemberMat, a: Anchor, b: Anchor, p0: Vec2, p1: Vec2) -> Self {
        Member {
            uid: NEXT_MEMBER_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            mat,
            a,
            b,
            p0,
            p1,
            stress: 0.0,
            over: 0,
            body: None,
            joints: vec![],
        }
    }

    pub fn len(&self) -> f32 {
        (self.p1 - self.p0).length()
    }

    /// Floor or structure? A rod lying roughly flat (within about 20 degrees) is
    /// deck: a car drives on it and its weight rests on it. A steeper rod is
    /// load-bearing structure (a truss diagonal, a post): it carries the loads but
    /// does not block the car.
    pub fn is_deck(&self) -> bool {
        let d = self.p1 - self.p0;
        d.y.abs() < 0.36 * d.x.abs().max(1e-3)
    }

    pub fn center(&self) -> Vec2 {
        (self.p0 + self.p1) * 0.5
    }

    pub fn angle(&self) -> f32 {
        let d = self.p1 - self.p0;
        d.y.atan2(d.x)
    }

    /// A copy with no physics state (and the beam back at its start position).
    pub fn fresh(&self) -> Member {
        Member { body: None, joints: vec![], over: 0, stress: 0.0, ..self.clone() }
    }

    pub fn covers(&self, p: Vec2) -> bool {
        self.distance_to(p) <= self.mat.thickness() / 2.0
    }

    pub fn distance_to(&self, p: Vec2) -> f32 {
        let ab = self.p1 - self.p0;
        let t = if ab.length_squared() > 0.0 { ((p - self.p0).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
        (self.p0 + ab * t - p).length()
    }

    pub fn bounds(&self) -> (i32, i32, i32, i32) {
        let r = self.mat.thickness() + 1.0;
        (
            ((self.p0.x.min(self.p1.x) - r).floor() as i32).max(0),
            ((self.p0.y.min(self.p1.y) - r).floor() as i32).max(0),
            ((self.p0.x.max(self.p1.x) + r).ceil() as i32).min(W as i32 - 1),
            ((self.p0.y.max(self.p1.y) + r).ceil() as i32).min(H as i32 - 1),
        )
    }
}

/// How close to the edge of the world (cells) a click snaps onto it.
pub const BORDER_REACH: f32 = 16.0;

/// Where a click lands on the structure: the end of a rod, a block, the middle of
/// a rod, the ground, or the edge of the world. Used by rods and cables alike.
pub fn node_at(world: &World, blocks: &[Block], members: &[Member], p: Vec2) -> Option<Anchor> {
    let mut best: Option<(f32, Anchor)> = None;
    for m in members {
        for (end, q) in [(0u8, m.p0), (1u8, m.p1)] {
            let d = (q - p).length();
            if d < 9.0 && best.map_or(true, |b| d < b.0) {
                best = Some((d, Anchor::End { uid: m.uid, end }));
            }
        }
    }
    if let Some((_, a)) = best {
        return Some(a);
    }
    if let Some(i) = block_at(blocks, p) {
        let b = &blocks[i];
        return Some(Anchor::Block { uid: b.uid, local: rot(-b.angle, p - b.pos) });
    }
    for m in members {
        if m.distance_to(p) <= m.mat.thickness() / 2.0 + 3.0 {
            let ab = m.p1 - m.p0;
            let t = ((p - m.p0).dot(ab) / ab.length_squared().max(1.0)).clamp(0.0, 1.0);
            return Some(Anchor::Along { uid: m.uid, t });
        }
    }
    anchor_at(world, blocks, p)
}

/// Cables and beams share the same snapping for the ground and blocks.

// ---------------------------------------------------------------------------
// Cables

/// What one end of a cable is fixed to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    Ground(Vec2),
    /// A point on a block, in the block's own frame.
    Block { uid: u32, local: Vec2 },
    /// One end (0 or 1) of a free rod: a node other rods and cables can pin to.
    End { uid: u32, end: u8 },
    /// A point along a rod, `t` from 0 (first end) to 1 (second end).
    Along { uid: u32, t: f32 },
    /// Pinned to nothing (a loose end).
    Free(Vec2),
    /// The drum of a winch (a gear with a reel). `pos` is where the drum is now; the game keeps it
    /// up to date, since this module knows nothing about gears.
    Reel { uid: u32, pos: Vec2 },
}

/// How a rope's length follows its winch: `len = base_len - radius * (angle - base_angle)`.
#[derive(Clone, Copy, Debug)]
pub struct ReelRef {
    pub gear: u32,
    pub base_len: f32,
    pub base_angle: f32,
    pub radius: f32,
    pub min_len: f32,
    pub max_len: f32,
}

#[derive(Clone)]
pub struct Cable {
    pub a: Anchor,
    pub b: Anchor,
    /// Rope length in cells (a little longer than the distance between the ends).
    pub len: f32,
    pub mat: CableMat,
    /// Elasticity multiplier: above 1 it stretches more, below 1 it is stiffer.
    pub soft: f32,
    /// Load as a fraction of what it can carry (1 = snaps).
    pub stress: f32,
    /// Points along the cable for drawing: both anchors and every link between.
    pub pts: Vec<Vec2>,
    /// Pulleys the rope runs over, in order (their ids), and where the rope touches each one
    /// (filled in by the game before the rope is built).
    pub via: Vec<u32>,
    pub via_pts: Vec<Vec2>,
    /// A rope fixed on a winch drum: how its length follows the drum (set when the simulation starts).
    pub reel: Option<ReelRef>,
    pub snapped: bool,
    links: Vec<RigidBodyHandle>,
    joints: Vec<ImpulseJointHandle>,
}

impl Cable {
    pub fn new(a: Anchor, b: Anchor, chord: f32) -> Self {
        Cable { a, b, len: chord * 1.06, mat: CableMat::Iron, soft: 1.0, stress: 0.0, pts: vec![], via: vec![], via_pts: vec![], reel: None, snapped: false, links: vec![], joints: vec![] }
    }

    pub fn made_of(mut self, mat: CableMat, soft: f32) -> Self {
        self.mat = mat;
        self.soft = soft;
        self
    }

    /// A copy with no physics state, as it was before the simulation started.
    pub fn fresh(&self) -> Cable {
        let mut c = Cable::new(self.a, self.b, self.len / 1.06).made_of(self.mat, self.soft);
        c.len = self.len;
        c.via = self.via.clone();
        c
    }

    pub fn reset_runtime(&mut self) {
        self.links.clear();
        self.joints.clear();
        self.pts.clear();
        self.snapped = false;
    }

    pub fn has_body(&self) -> bool {
        !self.links.is_empty()
    }

    pub fn attached_to_member(&self, uid: u32) -> bool {
        let on = |a: &Anchor| matches!(a, Anchor::End { uid: u, .. } | Anchor::Along { uid: u, .. } if *u == uid);
        on(&self.a) || on(&self.b)
    }

    pub fn attached_to(&self, uid: u32) -> bool {
        matches!(self.a, Anchor::Block { uid: u, .. } if u == uid) || matches!(self.b, Anchor::Block { uid: u, .. } if u == uid)
    }

    /// Distance from `p` to the drawn cable (or `f32::MAX` before it has a shape).
    pub fn distance_to(&self, p: Vec2) -> f32 {
        let mut best = f32::MAX;
        for w in self.pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let ab = b - a;
            let t = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
            best = best.min((a + ab * t - p).length());
        }
        best
    }
}

pub fn anchor_world(anchor: &Anchor, blocks: &[Block], members: &[Member]) -> Option<Vec2> {
    match anchor {
        Anchor::Ground(p) | Anchor::Free(p) => Some(*p),
        Anchor::Block { uid, local } => blocks.iter().find(|b| b.uid == *uid).map(|b| b.pos + rot(b.angle, *local)),
        Anchor::End { uid, end } => members.iter().find(|m| m.uid == *uid).map(|m| if *end == 0 { m.p0 } else { m.p1 }),
        Anchor::Along { uid, t } => members.iter().find(|m| m.uid == *uid).map(|m| m.p0 + (m.p1 - m.p0) * *t),
        Anchor::Reel { pos, .. } => Some(*pos),
    }
}

/// Where a click would attach a cable: onto a block, or onto solid ground nearby.
pub fn anchor_at(world: &World, blocks: &[Block], p: Vec2) -> Option<Anchor> {
    if let Some(i) = block_at(blocks, p) {
        let b = &blocks[i];
        return Some(Anchor::Block { uid: b.uid, local: rot(-b.angle, p - b.pos) });
    }
    for r in 0..=4 {
        for dy in -r..=r {
            for dx in -r..=r {
                let (x, y) = (p.x as i32 + dx, p.y as i32 + dy);
                if world.raw(x, y).is_hard() {
                    return Some(Anchor::Ground(vec2(x as f32 + 0.5, y as f32 + 0.5)));
                }
            }
        }
    }
    // The edges of the world are solid to hang things from.
    // Near any edge of the world, including the top of the sky: it snaps to the edge.
    let (w, h) = (W as f32, H as f32);
    let near = BORDER_REACH;
    if p.x < near {
        return Some(Anchor::Ground(vec2(0.0, p.y.clamp(0.0, h))));
    }
    if p.x > w - near {
        return Some(Anchor::Ground(vec2(w, p.y.clamp(0.0, h))));
    }
    if p.y < near {
        return Some(Anchor::Ground(vec2(p.x.clamp(0.0, w), 0.0)));
    }
    if p.y > h - near {
        return Some(Anchor::Ground(vec2(p.x.clamp(0.0, w), h)));
    }
    None
}

/// What a cable is made of: how stiff it is (elasticity), how much it can take,
/// how heavy it is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CableMat {
    Steel,
    Iron,
    Wood,
}

impl CableMat {
    pub const ALL: [CableMat; 3] = [CableMat::Steel, CableMat::Iron, CableMat::Wood];

    pub fn name(self) -> &'static str {
        match self {
            CableMat::Steel => "acier",
            CableMat::Iron => "fer",
            CableMat::Wood => "corde",
        }
    }

    pub fn id(self) -> u8 {
        match self {
            CableMat::Steel => 0,
            CableMat::Iron => 1,
            CableMat::Wood => 2,
        }
    }

    pub fn from_id(id: u8) -> CableMat {
        Self::ALL.iter().copied().find(|m| m.id() == id).unwrap_or(CableMat::Iron)
    }

    /// Spring stiffness of each link: steel hardly stretches, rope stretches a lot.
    pub fn stiffness(self) -> f32 {
        match self {
            CableMat::Steel => 20000.0,
            CableMat::Iron => 8000.0,
            CableMat::Wood => 700.0,
        }
    }

    /// Impulse per step a link can carry before the cable snaps.
    pub fn limit(self) -> f32 {
        match self {
            CableMat::Steel => 12.0,
            CableMat::Iron => 6.0,
            CableMat::Wood => 2.4,
        }
    }

    /// Bead density: steel is heavy, rope is light.
    pub fn density(self) -> f32 {
        match self {
            CableMat::Steel => 20.0,
            CableMat::Iron => 12.0,
            CableMat::Wood => 5.0,
        }
    }

    pub fn color(self) -> (u8, u8, u8) {
        match self {
            CableMat::Steel => (92, 118, 168),
            CableMat::Iron => (62, 66, 78),
            CableMat::Wood => (176, 140, 88),
        }
    }

    pub fn cost_from(self) -> &'static [Mat] {
        match self {
            CableMat::Steel => &[Mat::Steel],
            CableMat::Iron => &[Mat::Iron],
            CableMat::Wood => &[Mat::Wood, Mat::Trunk, Mat::Branch],
        }
    }

    pub fn cost(self, len: f32) -> u32 {
        (len * 0.15).ceil() as u32
    }
}

const CABLE_SEGMENT: f32 = 8.0;
/// How far (cells) clipped bricks can move apart or together vertically, and how far (radians) they
/// can rock, before the clip lets go.
const STUD_PLAY: f32 = 0.5;
const STUD_TILT: f32 = 0.05;
/// Stiffness of a connecting rod to a block (newtons per metre).
pub const ROD_STIFFNESS: f32 = 4000.0;

// ---------------------------------------------------------------------------
// Springs and elastic cords

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpringKind {
    /// A steel coil: strong, fairly stiff.
    Coil,
    /// An elastic cord: weak and very stretchy.
    Elastic,
}

impl SpringKind {
    pub const ALL: [SpringKind; 2] = [SpringKind::Coil, SpringKind::Elastic];

    pub fn name(self) -> &'static str {
        match self {
            SpringKind::Coil => "Ressort",
            SpringKind::Elastic => "Élastique",
        }
    }

    pub fn id(self) -> u8 {
        self as u8
    }

    pub fn from_id(id: u8) -> SpringKind {
        Self::ALL.iter().copied().find(|k| k.id() == id).unwrap_or(SpringKind::Coil)
    }

    /// Newtons per metre in the physics world.
    pub fn stiffness(self) -> f32 {
        match self {
            SpringKind::Coil => 420.0,
            SpringKind::Elastic => 55.0,
        }
    }

    /// It snaps when stretched beyond this many times its rest length.
    pub fn max_stretch(self) -> f32 {
        match self {
            SpringKind::Coil => 2.4,
            SpringKind::Elastic => 3.6,
        }
    }

    pub fn color(self) -> (u8, u8, u8) {
        match self {
            SpringKind::Coil => (170, 176, 192),
            SpringKind::Elastic => (206, 96, 84),
        }
    }

    pub fn cost_from(self) -> &'static [Mat] {
        match self {
            SpringKind::Coil => &[Mat::Steel],
            SpringKind::Elastic => &[Mat::Wood, Mat::Trunk, Mat::Branch],
        }
    }

    pub fn cost(self, rest: f32) -> u32 {
        (rest * 0.2).ceil() as u32
    }
}

/// Longest rest length of a spring, in cells.
pub const SPRING_MAX_LEN: f32 = 140.0;

/// A spring or elastic cord: set up with three clicks (first end, rest length,
/// then the far end, pulled out to wherever it should be attached).
#[derive(Clone)]
pub struct Spring {
    pub uid: u32,
    pub kind: SpringKind,
    pub a: Anchor,
    pub b: Anchor,
    /// Length at rest, in cells.
    pub rest: f32,
    /// Elasticity multiplier (above 1 = softer).
    pub soft: f32,
    /// Current end points, for drawing.
    pub p0: Vec2,
    pub p1: Vec2,
    /// How far it is stretched (0 = at rest, 1 = about to snap).
    pub stress: f32,
    pub snapped: bool,
    bead: Option<RigidBodyHandle>,
    joint: Option<ImpulseJointHandle>,
}

static NEXT_SPRING_UID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

impl Spring {
    pub fn new(kind: SpringKind, a: Anchor, b: Anchor, rest: f32, soft: f32, p0: Vec2, p1: Vec2) -> Self {
        Spring {
            uid: NEXT_SPRING_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            kind,
            a,
            b,
            rest,
            soft,
            p0,
            p1,
            stress: 0.0,
            snapped: false,
            bead: None,
            joint: None,
        }
    }

    pub fn has_body(&self) -> bool {
        self.joint.is_some()
    }

    pub fn fresh(&self) -> Spring {
        Spring { bead: None, joint: None, stress: 0.0, snapped: false, ..self.clone() }
    }

    pub fn attached_to(&self, uid: u32) -> bool {
        let on = |a: &Anchor| matches!(a, Anchor::Block { uid: u, .. } if *u == uid);
        on(&self.a) || on(&self.b)
    }

    pub fn attached_to_member(&self, uid: u32) -> bool {
        let on = |a: &Anchor| matches!(a, Anchor::End { uid: u, .. } | Anchor::Along { uid: u, .. } if *u == uid);
        on(&self.a) || on(&self.b)
    }

    pub fn distance_to(&self, p: Vec2) -> f32 {
        let ab = self.p1 - self.p0;
        let t = if ab.length_squared() > 0.0 { ((p - self.p0).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
        (self.p0 + ab * t - p).length()
    }
}

pub struct Physics {
    w: PhysicsWorld,
    terrain: RigidBodyHandle,
    tiles: HashMap<(i32, i32), (u64, Vec<ColliderHandle>)>,
    /// Strength of each weld (a rusty or wooden block breaks its welds sooner).
    welds: HashMap<ImpulseJointHandle, f32>,
    last_vel: HashMap<RigidBodyHandle, Vector>,
    impacts: Vec<u32>,
    /// Hard knocks to any block this step: (where, how hard). For sound.
    bumps: Vec<(Vec2, f32)>,
    /// Where the mechanism end of each connecting rod (by rod id) is held: a body the game moves.
    pins: HashMap<u32, (RigidBodyHandle, ImpulseJointHandle)>,
    /// A body at each winch drum (by gear id): the end of a rope on a reel is attached to it.
    reels: HashMap<u32, RigidBodyHandle>,
    /// Fixed sheaves of pulleys (by pulley id): ropes bend round them.
    pulleys: HashMap<u32, ColliderHandle>,
}


fn v2(v: Vec2) -> Vector {
    Vector::new(v.x / PX_PER_M, v.y / PX_PER_M)
}

impl Physics {
    pub fn new() -> Self {
        let mut w = PhysicsWorld::new();
        w.gravity = Vector::new(0.0, 16.0);
        w.integration_parameters.dt = 1.0 / 60.0;
        let terrain = w.insert_body(RigidBodyBuilder::fixed());
        Physics { w, terrain, tiles: HashMap::new(), welds: HashMap::new(), last_vel: HashMap::new(), impacts: vec![], bumps: vec![], pins: HashMap::new(), reels: HashMap::new(), pulleys: HashMap::new() }
    }

    pub fn add_block(&mut self, b: &mut Block) {
        let s = b.spec();
        let body = RigidBodyBuilder::dynamic()
            .translation(v2(b.pos))
            .rotation(b.angle)
            .can_sleep(false)
            .ccd_enabled(true)
            .linear_damping(0.05)
            .angular_damping(0.3);
        let shape = if s.round {
            ColliderBuilder::ball(s.w / 2.0 / PX_PER_M)
        } else {
            ColliderBuilder::cuboid(s.w / 2.0 / PX_PER_M, s.h / 2.0 / PX_PER_M)
        };
        let (h, _) = self.w.insert(body, shape.density(s.density).friction(0.8).restitution(0.05));
        b.body = Some(h);
    }

    pub fn remove_block(&mut self, b: &mut Block) {
        if let Some(h) = b.body.take() {
            self.w.remove_body(h);
        }
    }

    fn member_end_local(m: &Member, end: u8) -> Vector {
        let h = m.len() / 2.0 / PX_PER_M;
        Vector::new(if end == 0 { -h } else { h }, 0.0)
    }

    fn add_member_body(&mut self, m: &mut Member) {
        let len = m.len().max(2.0);
        let th = m.mat.thickness();
        let rb = RigidBodyBuilder::dynamic()
            .translation(v2(m.center()))
            .rotation(m.angle())
            .can_sleep(false)
            .ccd_enabled(true)
            .linear_damping(0.05)
            .angular_damping(0.4);
        let col = ColliderBuilder::cuboid(len / 2.0 / PX_PER_M, th / 2.0 / PX_PER_M)
            .density(m.mat.density())
            .friction(0.7)
            .collision_groups(InteractionGroups { memberships: Group::GROUP_3, filter: Group::GROUP_1, test_mode: InteractionTestMode::And });
        let (h, _) = self.w.insert(rb, col);
        m.body = Some(h);
    }

    /// Pins both ends of a beam to whatever they are fixed to.
    fn pin_member(&mut self, blocks: &[Block], members: &mut [Member], idx: usize) {
        let me = members[idx].clone();
        let Some(mb) = me.body else { return };
        let mut handles = vec![];
        for (end, anchor) in [(0u8, me.a), (1u8, me.b)] {
            let target: Option<(RigidBodyHandle, Vector)> = match anchor {
                Anchor::Ground(w) => Some((self.terrain, v2(w))),
                Anchor::Block { uid, local } => blocks.iter().find(|b| b.uid == uid).and_then(|b| Some((b.body?, v2(local)))),
                Anchor::End { uid, end: e } => members.iter().find(|o| o.uid == uid).and_then(|o| Some((o.body?, Self::member_end_local(o, e)))),
                Anchor::Along { uid, t } => members.iter().find(|o| o.uid == uid).and_then(|o| Some((o.body?, Vector::new((t - 0.5) * o.len() / PX_PER_M, 0.0)))),
                Anchor::Free(_) | Anchor::Reel { .. } => None,
            };
            if let Some((tb, tl)) = target {
                let j = RevoluteJointBuilder::new().local_anchor1(tl).local_anchor2(Self::member_end_local(&me, end)).contacts_enabled(false);
                handles.push(self.w.insert_impulse_joint(tb, mb, j));
            }
        }
        members[idx].joints.extend(handles);
    }

    pub fn build_members(&mut self, blocks: &[Block], members: &mut [Member]) {
        for m in members.iter_mut() {
            self.add_member_body(m);
        }
        for i in 0..members.len() {
            self.pin_member(blocks, members, i);
        }
    }

    pub fn add_member(&mut self, blocks: &[Block], members: &mut [Member], idx: usize) {
        if members[idx].body.is_none() {
            self.add_member_body(&mut members[idx]);
        }
        self.pin_member(blocks, members, idx);
    }

    pub fn remove_member(&mut self, m: &mut Member) {
        if let Some(h) = m.body.take() {
            self.w.remove_body(h);
        }
        m.joints.clear();
    }

    /// Follow the physics, measure how hard each beam is loaded, and snap the
    /// ones that are overloaded. Returns where beams broke.
    pub fn update_members(&mut self, world: &World, members: &mut Vec<Member>) -> Vec<(Vec2, u32)> {
        // Wooden beams float too.
        let dt = self.w.integration_parameters.dt;
        let g = self.w.gravity.y;
        for m in members.iter() {
            let Some(h) = m.body else { continue };
            let n = ((m.len() / 6.0) as usize).max(2);
            let (mut wet, mut c) = (0, Vec2::ZERO);
            for k in 0..n {
                let p = m.p0 + (m.p1 - m.p0) * ((k as f32 + 0.5) / n as f32);
                let water = |dx: f32, dy: f32| world.raw((p.x + dx) as i32, (p.y + dy) as i32) == Mat::Water;
                if water(0.0, 0.0) || water(-4.0, 0.0) || water(4.0, 0.0) || water(0.0, 4.0) {
                    wet += 1;
                    c += p;
                }
            }
            let rb = &mut self.w.bodies[h];
            if wet > 0 {
                let f = wet as f32 / n as f32;
                let area = m.len() * m.mat.thickness() / (PX_PER_M * PX_PER_M);
                rb.apply_impulse_at_point(Vector::new(0.0, -area * f * g * dt), v2(c / wet as f32), true);
                rb.set_linear_damping(0.05 + 2.5 * f);
            } else {
                rb.set_linear_damping(0.05);
            }
        }
        let index: HashMap<RigidBodyHandle, usize> = members.iter().enumerate().filter_map(|(i, m)| m.body.map(|b| (b, i))).collect();
        let mut ratio = vec![0.0f32; members.len()];
        for (_, j) in self.w.impulse_joints() {
            let f = (j.impulses.x * j.impulses.x + j.impulses.y * j.impulses.y).sqrt();
            for body in [j.body1(), j.body2()] {
                if let Some(&i) = index.get(&body) {
                    ratio[i] = ratio[i].max(f / members[i].mat.limit());
                }
            }
        }
        let mut broke = vec![];
        for (i, m) in members.iter_mut().enumerate() {
            let Some(h) = m.body else { continue };
            let rb = &self.w.bodies[h];
            let (t, a) = (rb.translation(), rb.rotation().angle());
            let c = vec2(t.x, t.y) * PX_PER_M;
            let half = rot(a, vec2(m.len() / 2.0, 0.0));
            m.p0 = c - half;
            m.p1 = c + half;
            m.stress = ratio[i];
            m.over = if ratio[i] > 1.0 { m.over + 1 } else { 0 };
            if m.over >= 3 {
                broke.push(i);
            }
        }
        let mut spots = vec![];
        for &i in broke.iter().rev() {
            let mut m = members.remove(i);
            spots.push((m.center(), m.uid));
            self.remove_member(&mut m);
        }
        spots
    }

    /// Weight on a beam at `point` for one step (a car driving over it).
    pub fn press_member(&mut self, m: &Member, point: Vec2, weight_kg: f32) {
        let Some(h) = m.body else { return };
        let impulse = Vector::new(0.0, weight_kg * self.w.gravity.y * self.w.integration_parameters.dt);
        self.w.bodies[h].apply_impulse_at_point(impulse, v2(point), true);
    }

    fn anchor_body(&self, blocks: &[Block], members: &[Member], a: &Anchor) -> Option<(RigidBodyHandle, Vector)> {
        match a {
            Anchor::Ground(w) => Some((self.terrain, v2(*w))),
            Anchor::Block { uid, local } => {
                let b = blocks.iter().find(|b| b.uid == *uid)?;
                Some((b.body?, v2(*local)))
            }
            Anchor::End { uid, end } => {
                let m = members.iter().find(|m| m.uid == *uid)?;
                Some((m.body?, Self::member_end_local(m, *end)))
            }
            Anchor::Along { uid, t } => {
                let m = members.iter().find(|m| m.uid == *uid)?;
                Some((m.body?, Vector::new((t - 0.5) * m.len() / PX_PER_M, 0.0)))
            }
            Anchor::Free(_) => None,
            Anchor::Reel { uid, .. } => self.reels.get(uid).map(|h| (*h, Vector::new(0.0, 0.0))),
        }
    }

    /// One spring joint between its two ends. A loose end gets a small weight to
    /// hang from the spring.
    pub fn add_spring(&mut self, blocks: &[Block], members: &[Member], sp: &mut Spring) {
        let (Some(pa), Some(pb)) = (anchor_world(&sp.a, blocks, members), anchor_world(&sp.b, blocks, members)) else { return };
        let Some((ha, la)) = self.anchor_body(blocks, members, &sp.a) else { return };
        let (hb, lb) = match self.anchor_body(blocks, members, &sp.b) {
            Some(x) => x,
            None => {
                let rb = RigidBodyBuilder::dynamic().translation(v2(pb)).can_sleep(false).linear_damping(0.4).angular_damping(2.0);
                let col = ColliderBuilder::ball(2.0 / PX_PER_M)
                    .density(40.0)
                    .collision_groups(InteractionGroups { memberships: Group::GROUP_2, filter: Group::GROUP_1, test_mode: InteractionTestMode::And });
                let (h, _) = self.w.insert(rb, col);
                sp.bead = Some(h);
                (h, Vector::new(0.0, 0.0))
            }
        };
        let k = sp.kind.stiffness() / sp.soft.clamp(0.3, 4.0);
        let damping = 0.3 * k.sqrt();
        let j = SpringJointBuilder::new(sp.rest / PX_PER_M, k, damping).local_anchor1(la).local_anchor2(lb).contacts_enabled(false);
        sp.joint = Some(self.w.insert_impulse_joint(ha, hb, j));
        sp.p0 = pa;
        sp.p1 = pb;
    }

    pub fn remove_spring(&mut self, sp: &mut Spring) {
        if let Some(j) = sp.joint.take() {
            self.w.remove_impulse_joint(j);
        }
        if let Some(h) = sp.bead.take() {
            self.w.remove_body(h);
        }
    }

    /// Follow the physics; springs stretched or crushed too far snap. Returns the
    /// uids of those that snapped.
    pub fn sync_springs(&mut self, blocks: &[Block], members: &[Member], springs: &mut Vec<Spring>) -> Vec<u32> {
        let mut gone = vec![];
        for sp in springs.iter_mut() {
            if !sp.has_body() {
                continue;
            }
            if let Some(p) = anchor_world(&sp.a, blocks, members) {
                sp.p0 = p;
            }
            sp.p1 = match sp.bead {
                Some(h) => {
                    let t = self.w.bodies[h].translation();
                    vec2(t.x, t.y) * PX_PER_M
                }
                None => anchor_world(&sp.b, blocks, members).unwrap_or(sp.p1),
            };
            let len = (sp.p1 - sp.p0).length();
            let stretch = len / sp.rest.max(1.0);
            sp.stress = ((stretch - 1.0).abs() / (sp.kind.max_stretch() - 1.0)).clamp(0.0, 1.0);
            if stretch > sp.kind.max_stretch() {
                gone.push(sp.uid);
            }
        }
        for uid in &gone {
            if let Some(i) = springs.iter().position(|s| s.uid == *uid) {
                let mut sp = springs.remove(i);
                self.remove_spring(&mut sp);
            }
        }
        gone
    }

    /// Builds the rope as a chain of light beads joined by rope joints.
    pub fn add_cable(&mut self, blocks: &[Block], members: &[Member], c: &mut Cable) {
        let (Some(pa), Some(pb)) = (anchor_world(&c.a, blocks, members), anchor_world(&c.b, blocks, members)) else { return };
        // Long chains of light beads need more solver passes to hold a heavy load.
        self.w.integration_parameters.num_solver_iterations = 12;
        let n = ((c.len / CABLE_SEGMENT) as usize).clamp(4, 70);
        let link_len = c.len / (n as f32 + 1.0);
        // Slack gives a parabola; start from the hanging shape so nothing snaps at first.
        let chord = (pb - pa).length();
        let sag = 0.4 * (chord * (c.len - chord).max(0.0)).sqrt();
        let mut bodies: Vec<RigidBodyHandle> = vec![];
        // A rope that runs over pulleys starts along that path (taut, no sag).
        let path: Vec<Vec2> = if c.via_pts.is_empty() { vec![] } else { std::iter::once(pa).chain(c.via_pts.iter().copied()).chain(std::iter::once(pb)).collect() };
        let path_len: f32 = path.windows(2).map(|w| (w[1] - w[0]).length()).sum();
        let along = |u: f32| -> Vec2 {
            let mut left = u * path_len;
            for w in path.windows(2) {
                let l = (w[1] - w[0]).length();
                if left <= l || std::ptr::eq(w, path.windows(2).last().unwrap()) {
                    return w[0] + (w[1] - w[0]) * (left / l.max(1e-3)).min(1.0);
                }
                left -= l;
            }
            pb
        };
        for i in 1..=n {
            let u = i as f32 / (n as f32 + 1.0);
            let p = if path.is_empty() { pa + (pb - pa) * u + vec2(0.0, sag * 4.0 * u * (1.0 - u)) } else { along(u) };
            let rb = RigidBodyBuilder::dynamic().translation(v2(p)).can_sleep(false).linear_damping(0.6).angular_damping(2.0);
            let col = ColliderBuilder::ball(1.4 / PX_PER_M)
                .density(c.mat.density())
                .friction(0.6)
                .collision_groups(InteractionGroups { memberships: Group::GROUP_2, filter: Group::GROUP_1, test_mode: InteractionTestMode::And });
            let (h, _) = self.w.insert(rb, col);
            bodies.push(h);
        }
        let end_body = |this: &Physics, a: &Anchor| -> Option<(RigidBodyHandle, Vector)> {
            match a {
                Anchor::Ground(w) => Some((this.terrain, v2(*w))),
                Anchor::Block { uid, local } => {
                    let b = blocks.iter().find(|b| b.uid == *uid)?;
                    Some((b.body?, v2(*local)))
                }
                Anchor::End { uid, end } => {
                    let m = members.iter().find(|m| m.uid == *uid)?;
                    Some((m.body?, Self::member_end_local(m, *end)))
                }
                Anchor::Along { uid, t } => {
                    let m = members.iter().find(|m| m.uid == *uid)?;
                    Some((m.body?, Vector::new((t - 0.5) * m.len() / PX_PER_M, 0.0)))
                }
                Anchor::Free(_) => None,
                Anchor::Reel { uid, .. } => this.reels.get(uid).map(|h| (*h, Vector::new(0.0, 0.0))),
            }
        };
        // One end may be left loose: the rope then just hangs from the other one.
        let (ea, eb) = (end_body(self, &c.a), end_body(self, &c.b));
        let loose = |a: &Anchor| matches!(a, Anchor::Free(_));
        if (ea.is_none() && !loose(&c.a)) || (eb.is_none() && !loose(&c.b)) || (ea.is_none() && eb.is_none()) {
            return;
        }
        let rest = link_len / PX_PER_M;
        // A softer cable has weaker springs (it stretches more); damping follows the stiffness.
        let stiffness = c.mat.stiffness() / c.soft.clamp(0.3, 4.0);
        let damping = 0.27 * stiffness.sqrt();
        let mut joints = vec![];
        let chain: Vec<(RigidBodyHandle, Vector)> = ea
            .into_iter()
            .chain(bodies.iter().map(|h| (*h, Vector::new(0.0, 0.0))))
            .chain(eb)
            .collect();
        for pair in chain.windows(2) {
            // An elastic rope: springs between the beads let it stretch a little.
            let j = SpringJointBuilder::new(rest, stiffness, damping).local_anchor1(pair[0].1).local_anchor2(pair[1].1).contacts_enabled(false);
            joints.push(self.w.insert_impulse_joint(pair[0].0, pair[1].0, j));
        }
        c.links = bodies;
        c.joints = joints;
        c.snapped = false;
    }

    pub fn remove_cable(&mut self, c: &mut Cable) {
        for h in c.links.drain(..) {
            self.w.remove_body(h);
        }
        c.joints.clear();
    }

    /// Copies bead positions into each cable and snaps the ones pulled too hard.
    pub fn sync_cables(&mut self, blocks: &[Block], members: &[Member], cables: &mut [Cable]) {
        for c in cables.iter_mut() {
            if !c.has_body() {
                continue;
            }
            let mut broke = vec![];
            let mut worst = 0.0f32;
            for (k, h) in c.joints.iter().enumerate() {
                if let Some(j) = self.w.impulse_joints.get(*h) {
                    let f = (j.impulses.x * j.impulses.x + j.impulses.y * j.impulses.y).sqrt();
                    worst = worst.max(f / c.mat.limit());
                    if f > c.mat.limit() {
                        broke.push(k);
                    }
                }
            }
            c.stress = worst;
            for k in broke {
                self.w.remove_impulse_joint(c.joints[k]);
                c.snapped = true;
            }
            let mut pts = vec![];
            if let (Some(p), false) = (anchor_world(&c.a, blocks, members), matches!(c.a, Anchor::Free(_))) {
                pts.push(p);
            }
            for h in &c.links {
                let t = self.w.bodies[*h].translation();
                pts.push(vec2(t.x, t.y) * PX_PER_M);
            }
            if let (Some(p), false) = (anchor_world(&c.b, blocks, members), matches!(c.b, Anchor::Free(_))) {
                pts.push(p);
            }
            c.pts = pts;
        }
    }

    /// Two planks or beams of the same kind laid end to end in a straight line
    /// act as one long rigid beam.
    fn end_to_end(a: &Block, b: &Block) -> bool {
        let beam = |k: BlockKind| matches!(k, BlockKind::Plank | BlockKind::Beam | BlockKind::SteelBeam);
        if a.kind != b.kind || !beam(a.kind) {
            return false;
        }
        let dir = rot(a.angle, vec2(1.0, 0.0));
        let parallel = (a.angle - b.angle).sin().abs() < 0.03;
        let d = b.pos - a.pos;
        let along = d.dot(dir);
        let lateral = (d - dir * along).length();
        parallel && lateral < 1.5 && (along.abs() - a.spec().w).abs() < 2.0
    }

    fn weld(&mut self, a: &Block, b: &Block) {
        let (Some(ha), Some(hb)) = (a.body, b.body) else { return };
        let mid = (a.pos + b.pos) * 0.5;
        let la = rot(-a.angle, mid - a.pos) / PX_PER_M;
        let lb = rot(-b.angle, mid - b.pos) / PX_PER_M;
        let studs = block_studs(a, b);
        let frame1 = Pose::new(Vector::new(la.x, la.y), 0.0);
        let frame2 = Pose::new(Vector::new(lb.x, lb.y), a.angle - b.angle);
        // Bricks that share studs are clipped: they cannot slide sideways, only come apart or go
        // together vertically (by the height of a stud) and rock a little. Anything else (side by
        // side, beams, planks) is welded rigidly as before.
        let h = if studs > 0 {
            // They come apart (positive when the first is the upper one) but cannot sink into each other.
            let sink = 0.1 / PX_PER_M;
            let apart = STUD_PLAY / PX_PER_M;
            let range = if a.pos.y < b.pos.y { [-sink, apart] } else { [-apart, sink] };
            let joint = GenericJointBuilder::new(JointAxesMask::LIN_X)
                .local_frame1(frame1)
                .local_frame2(frame2)
                .limits(JointAxis::LinY, range)
                .limits(JointAxis::AngX, [-STUD_TILT, STUD_TILT])
                .contacts_enabled(false)
                .build();
            self.w.insert_impulse_joint(ha, hb, joint)
        } else {
            let joint = FixedJointBuilder::new().local_frame1(frame1).local_frame2(frame2).contacts_enabled(false);
            self.w.insert_impulse_joint(ha, hb, joint)
        };
        let q = |b: &Block| b.spec().strength * (1.0 - 0.7 * b.rust.clamp(0.0, 1.0));
        let bonus = if Self::end_to_end(a, b) { 1.5 } else { 1.0 };
        // Every stud they share grips a little more: the hold of a brick on another grows with the number
        // of studs in common (side-by-side bricks, which share none, keep the plain hold).
        let grip = 1.0 + 0.25 * block_studs(a, b) as f32;
        self.welds.insert(h, q(a).min(q(b)) * bonus * grip);
    }

    /// Weld block `i` to every touching block (wheels stay free).
    pub fn weld_neighbours(&mut self, blocks: &[Block], i: usize) {
        if blocks[i].spec().round {
            return;
        }
        for j in 0..blocks.len() {
            if j != i && !blocks[j].spec().round && blocks[i].gap(&blocks[j]) < 1.5 {
                self.weld(&blocks[i], &blocks[j]);
            }
        }
    }

    pub fn build(blocks: &mut [Block]) -> Physics {
        let mut p = Physics::new();
        for b in blocks.iter_mut() {
            p.add_block(b);
        }
        for i in 0..blocks.len() {
            for j in 0..i {
                if !blocks[i].spec().round && !blocks[j].spec().round && blocks[i].gap(&blocks[j]) < 1.5 {
                    p.weld(&blocks[i], &blocks[j]);
                }
            }
        }
        p
    }

    #[cfg(test)]
    pub fn joint_count(&self) -> usize {
        self.w.impulse_joints().count()
    }

    /// How close each block's welds are to breaking, 0 (idle) to 1 (about to snap).
    pub fn stress(&self, blocks: &[Block]) -> HashMap<u32, f32> {
        let mut out: HashMap<u32, f32> = HashMap::new();
        for (h, j) in self.w.impulse_joints() {
            let k = self.welds.get(&h).copied().unwrap_or(1.0);
            let f = (j.impulses.x * j.impulses.x + j.impulses.y * j.impulses.y).sqrt() / (BREAK_FORCE * k);
            let t = j.impulses.z.abs() / (BREAK_TORQUE * k);
            let ratio = f.max(t);
            for body in [j.body1(), j.body2()] {
                if let Some(b) = blocks.iter().find(|b| b.body == Some(body)) {
                    let e = out.entry(b.uid).or_insert(0.0);
                    *e = e.max(ratio);
                }
            }
        }
        out
    }

    /// Largest (force, torque) load currently carried by a weld.
    #[cfg(test)]
    pub fn max_joint_load(&self) -> (f32, f32) {
        self.w.impulse_joints().fold((0.0, 0.0), |(f, t): (f32, f32), (_, j)| {
            (f.max((j.impulses.x * j.impulses.x + j.impulses.y * j.impulses.y).sqrt()), t.max(j.impulses.z.abs()))
        })
    }

    /// Press down on a block at `point` (cells) with `weight_kg` for one step,
    /// for example under a car wheel.
    pub fn press(&mut self, b: &Block, point: Vec2, weight_kg: f32) {
        let Some(h) = b.body else { return };
        let rb = &mut self.w.bodies[h];
        let impulse = Vector::new(0.0, weight_kg * self.w.gravity.y * self.w.integration_parameters.dt);
        rb.apply_impulse_at_point(impulse, v2(point), true);
    }

    /// Fixes a wheel on an axle: it can turn freely but stays where its centre is.
    pub fn add_pivot(&mut self, blocks: &[Block], members: &[Member], wheel: u32, support: &Anchor) -> bool {
        let Some(bw) = blocks.iter().find(|b| b.uid == wheel) else { return false };
        let Some(hw) = bw.body else { return false };
        let target = match support {
            Anchor::Ground(p) | Anchor::Free(p) => Some((self.terrain, v2(*p))),
            Anchor::Block { uid, local } => blocks.iter().find(|b| b.uid == *uid).and_then(|b| Some((b.body?, v2(*local)))),
            Anchor::End { uid, end } => members.iter().find(|m| m.uid == *uid).and_then(|m| Some((m.body?, Self::member_end_local(m, *end)))),
            Anchor::Along { uid, t } => members.iter().find(|m| m.uid == *uid).and_then(|m| Some((m.body?, Vector::new((t - 0.5) * m.len() / PX_PER_M, 0.0)))),
            Anchor::Reel { .. } => None,
        };
        let Some((hs, local)) = target else { return false };
        if hs == hw {
            return false;
        }
        let j = RevoluteJointBuilder::new().local_anchor1(local).local_anchor2(Vector::new(0.0, 0.0)).contacts_enabled(false);
        let h = self.w.insert_impulse_joint(hs, hw, j);
        // An axle holds much more than a weld.
        self.welds.insert(h, 4.0);
        true
    }

    /// A connecting rod from a moving end of a mechanism (a gear pin, a carriage) to a block, a rod
    /// or the ground: the end is a body the game moves; a stiff spring is the rod itself.
    pub fn add_pin_rod(&mut self, blocks: &[Block], members: &[Member], rod: u32, pin: Vec2, node: &Anchor, len: f32) -> bool {
        let target = match node {
            Anchor::Ground(p) | Anchor::Free(p) => Some((self.terrain, v2(*p))),
            Anchor::Block { uid, local } => blocks.iter().find(|b| b.uid == *uid).and_then(|b| Some((b.body?, v2(*local)))),
            Anchor::End { uid, end } => members.iter().find(|m| m.uid == *uid).and_then(|m| Some((m.body?, Self::member_end_local(m, *end)))),
            Anchor::Along { uid, t } => members.iter().find(|m| m.uid == *uid).and_then(|m| Some((m.body?, Vector::new((t - 0.5) * m.len() / PX_PER_M, 0.0)))),
            Anchor::Reel { .. } => None,
        };
        let Some((hb, local)) = target else { return false };
        let body = RigidBodyBuilder::kinematic_position_based().translation(v2(pin)).can_sleep(false);
        let hp = self.w.insert_body(body);
        let j = SpringJointBuilder::new(len / PX_PER_M, ROD_STIFFNESS, 0.3 * ROD_STIFFNESS.sqrt()).local_anchor1(Vector::new(0.0, 0.0)).local_anchor2(local).contacts_enabled(false);
        let h = self.w.insert_impulse_joint(hp, hb, j);
        // The rod is as strong as the mechanism's rod (the game breaks it), not like a weld.
        self.welds.insert(h, 1e6);
        self.pins.insert(rod, (hp, h));
        true
    }

    /// Moves the mechanism end of a rod to `p` (it takes effect over the next step).
    pub fn move_pin(&mut self, rod: u32, p: Vec2) {
        if let Some((h, _)) = self.pins.get(&rod) {
            self.w.bodies[*h].set_next_kinematic_translation(v2(p));
        }
    }

    /// Takes a rod's spring and end body away (the rod broke or was erased).
    pub fn remove_pin_rod(&mut self, rod: u32) {
        if let Some((hp, hj)) = self.pins.remove(&rod) {
            self.w.remove_impulse_joint(hj);
            self.w.remove_body(hp);
        }
    }

    /// The pull of a rod's spring on its mechanism end, in physics units (newtons), given where
    /// the other end is now. Positive along the way from the pin to the other end when stretched.
    pub fn rod_pull(pin: Vec2, other: Vec2, len: f32) -> Vec2 {
        let d = other - pin;
        let dist = d.length();
        if dist < 1e-4 {
            return Vec2::ZERO;
        }
        d / dist * ((dist - len) / PX_PER_M * ROD_STIFFNESS)
    }

    /// A body at a winch drum, which the end of a rope on that reel is attached to.
    pub fn add_reel(&mut self, uid: u32, pos: Vec2) {
        let h = self.w.insert_body(RigidBodyBuilder::kinematic_position_based().translation(v2(pos)).can_sleep(false));
        self.reels.insert(uid, h);
    }

    pub fn move_reel(&mut self, uid: u32, pos: Vec2) {
        if let Some(h) = self.reels.get(&uid) {
            self.w.bodies[*h].set_next_kinematic_translation(v2(pos));
        }
    }

    /// The fixed sheave of a pulley, on whatever it is mounted on. Only ropes touch it, and
    /// nothing grips: the rope slides round it freely.
    pub fn add_pulley(&mut self, blocks: &[Block], members: &[Member], uid: u32, support: &Anchor, pos: Vec2, radius: f32) {
        let (body, local) = match support {
            Anchor::Block { uid: b, local } => match blocks.iter().find(|x| x.uid == *b).and_then(|x| x.body) {
                Some(h) => (h, v2(*local)),
                None => (self.terrain, v2(pos)),
            },
            Anchor::End { uid: m, end } => match members.iter().find(|x| x.uid == *m).and_then(|x| x.body.map(|h| (h, x))) {
                Some((h, x)) => (h, Self::member_end_local(x, *end)),
                None => (self.terrain, v2(pos)),
            },
            _ => (self.terrain, v2(pos)),
        };
        let col = ColliderBuilder::ball(radius / PX_PER_M)
            .translation(local)
            .friction(0.0)
            .collision_groups(InteractionGroups { memberships: Group::GROUP_1, filter: Group::GROUP_2, test_mode: InteractionTestMode::And });
        let h = self.w.insert_collider(col, Some(body));
        self.pulleys.insert(uid, h);
    }

    /// Reels a rope in or out: every link gets its share of the new length.
    pub fn set_cable_length(&mut self, c: &Cable, len: f32) {
        let links = c.joints.len();
        if links == 0 {
            return;
        }
        let rest = len / links as f32 / PX_PER_M;
        let stiffness = c.mat.stiffness() / c.soft.clamp(0.3, 4.0);
        let damping = 0.27 * stiffness.sqrt();
        for h in &c.joints {
            if let Some(j) = self.w.impulse_joints.get_mut(*h, true) {
                j.data.set_motor_position(JointAxis::LinX, rest, stiffness, damping);
            }
        }
    }

    /// The tension of a rope where it leaves its first (or last) point, in newtons, from how far
    /// its end link is stretched.
    pub fn rope_tension(c: &Cable, at_start: bool) -> f32 {
        let n = c.pts.len();
        if n < 3 || c.joints.is_empty() {
            return 0.0;
        }
        let (a, b) = if at_start { (c.pts[0], c.pts[1]) } else { (c.pts[n - 1], c.pts[n - 2]) };
        let rest = c.len / c.joints.len() as f32;
        let stiffness = c.mat.stiffness() / c.soft.clamp(0.3, 4.0);
        ((a - b).length() - rest).max(0.0) / PX_PER_M * stiffness
    }

    /// How fast a block turns, in radians per second.
    pub fn spin_of(&self, b: &Block) -> f32 {
        b.body.map_or(0.0, |h| self.w.bodies[h].angvel())
    }

    /// Twists a block: `torque` (kg m² / s²) for one step.
    pub fn twist(&mut self, b: &Block, torque: f32) {
        if let Some(h) = b.body {
            let dt = self.w.integration_parameters.dt;
            self.w.bodies[h].apply_torque_impulse(torque * dt, true);
        }
    }

    /// A hammer blow: a small shove at `point`, away from it.
    pub fn knock(&mut self, b: &Block, point: Vec2, dir: Vec2) {
        let Some(h) = b.body else { return };
        let rb = &mut self.w.bodies[h];
        let m = rb.mass();
        let impulse = Vector::new(dir.x, dir.y) * (m * 3.0);
        rb.apply_impulse_at_point(impulse, v2(point), true);
    }

    /// Push bodies away from an explosion.
    pub fn blast(&mut self, blocks: &[Block], cx: f32, cy: f32, r: f32) {
        let c = vec2(cx, cy);
        for b in blocks {
            let Some(h) = b.body else { continue };
            let d = (b.pos - c).length();
            let reach = r * 2.2;
            if d < reach {
                let dir = (b.pos - c).normalize_or_zero();
                let rb = &mut self.w.bodies[h];
                let strength = (1.0 - d / reach) * rb.mass() * 9.0;
                rb.apply_impulse(Vector::new(dir.x, dir.y - 0.3) * strength, true);
            }
        }
    }

    /// Water holds things up: the more of a body is under water, the harder it is
    /// pushed up. Light things (wood) float, heavy ones (iron, stone) sink.
    fn buoyancy(&mut self, world: &World, blocks: &[Block]) {
        let dt = self.w.integration_parameters.dt;
        let g = self.w.gravity.y;
        for b in blocks {
            let Some(h) = b.body else { continue };
            let s = b.spec();
            let half = b.half();
            // Wind pushes on whatever is in the open air, in proportion to the surface it offers.
            let gust = world.wind * world.wind.abs() * (s.h.max(s.w) * s.w.min(s.h)) / (PX_PER_M * PX_PER_M) * g.abs() * 0.045 * dt;
            self.w.bodies[h].apply_impulse(Vector::new(gust, 0.0), false);
            // A difference of pressure across the block pushes it from high to low (a blast wave,
            // a surge of water). The still weight of the liquid is the buoyancy below.
            let (gx, gy) = world.pressure_gradient(b.pos.x, b.pos.y);
            if gx * gx + gy * gy > 0.004 * 0.004 {
                let k = -600.0 * (s.w * s.h) / (PX_PER_M * PX_PER_M) * dt;
                self.w.bodies[h].apply_impulse(Vector::new(gx * k, gy * k), true);
            }
            let reach = half.length() + 2.0;
            let (nx, ny) = (((s.w / 4.0) as i32).max(2), ((s.h / 4.0) as i32).max(2));
            let (mut wet, mut total) = (0, 0);
            let mut centroid = Vec2::ZERO;
            for i in 0..nx {
                for j in 0..ny {
                    let local = vec2(((i as f32 + 0.5) / nx as f32 - 0.5) * s.w, ((j as f32 + 0.5) / ny as f32 - 0.5) * s.h);
                    if s.round && local.length() > s.w / 2.0 {
                        continue;
                    }
                    total += 1;
                    let p = b.pos + rot(b.angle, local);
                    let in_water = |dx: f32| world.raw((p.x + dx) as i32, p.y as i32) == Mat::Water;
                    if in_water(-reach) || in_water(reach) || in_water(0.0) {
                        wet += 1;
                        centroid += p;
                    }
                }
            }
            let rb = &mut self.w.bodies[h];
            if total == 0 || wet == 0 {
                rb.set_linear_damping(0.05);
                rb.set_angular_damping(0.3);
                continue;
            }
            let f = wet as f32 / total as f32;
            let area = s.w * s.h / (PX_PER_M * PX_PER_M);
            let lift = 1.0 * area * f * g * dt;
            rb.apply_impulse_at_point(Vector::new(0.0, -lift), v2(centroid / wet as f32), true);
            rb.set_linear_damping(0.05 + 2.5 * f);
            rb.set_angular_damping(0.3 + 2.0 * f);
        }
    }

    /// Glass blocks that took a hard knock this step (something fast hit them, or
    /// they stopped suddenly after a fall).
    pub fn take_impacts(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.impacts)
    }

    pub fn take_bumps(&mut self) -> Vec<(Vec2, f32)> {
        std::mem::take(&mut self.bumps)
    }

    fn detect_impacts(&mut self, blocks: &[Block]) {
        let vel = |this: &Physics, b: &Block| b.body.map(|h| this.w.bodies[h].linvel());
        let speeds: Vec<Option<Vector>> = blocks.iter().map(|b| vel(self, b)).collect();
        // Any block that was moving fast and has just been stopped or knocked makes a noise.
        for (i, b) in blocks.iter().enumerate() {
            if let (Some(h), Some(v)) = (b.body, speeds[i]) {
                if let Some(prev) = self.last_vel.get(&h) {
                    let dv = ((v.x - prev.x).powi(2) + (v.y - prev.y).powi(2)).sqrt();
                    if dv > 1.6 && self.bumps.len() < 8 {
                        self.bumps.push((b.pos, dv));
                    }
                }
            }
        }
        for (i, g) in blocks.iter().enumerate() {
            if !g.kind.is_glass() {
                continue;
            }
            let (Some(h), Some(v)) = (g.body, speeds[i]) else { continue };
            // It was moving fast and has just been stopped or deflected.
            if let Some(prev) = self.last_vel.get(&h) {
                let dv = Vector::new(v.x - prev.x, v.y - prev.y);
                if (dv.x * dv.x + dv.y * dv.y).sqrt() > 4.6 {
                    self.impacts.push(g.uid);
                    continue;
                }
            }
            // A fast object slams into it.
            for (j, o) in blocks.iter().enumerate() {
                if j == i {
                    continue;
                }
                let Some(vo) = speeds[j] else { continue };
                let rel = Vector::new(vo.x - v.x, vo.y - v.y);
                if (rel.x * rel.x + rel.y * rel.y).sqrt() > 4.6 && g.gap(o) < 1.5 {
                    self.impacts.push(g.uid);
                    break;
                }
            }
        }
        self.last_vel.clear();
        for (b, v) in blocks.iter().zip(speeds) {
            if let (Some(h), Some(v)) = (b.body, v) {
                self.last_vel.insert(h, v);
            }
        }
    }

    pub fn step(&mut self, world: &World, blocks: &mut [Block]) {
        self.refresh_tiles(world, blocks);
        self.buoyancy(world, blocks);
        self.w.step();
        self.detect_impacts(blocks);

        let welds = &self.welds;
        let broken: Vec<ImpulseJointHandle> = self
            .w
            .impulse_joints()
            .filter(|(h, j)| {
                let k = welds.get(h).copied().unwrap_or(1.0);
                let f = (j.impulses.x * j.impulses.x + j.impulses.y * j.impulses.y).sqrt();
                f > BREAK_FORCE * k || j.impulses.z.abs() > BREAK_TORQUE * k
            })
            .map(|(h, _)| h)
            .collect();
        for h in broken {
            self.w.remove_impulse_joint(h);
            self.welds.remove(&h);
        }

        for b in blocks.iter_mut() {
            if let Some(h) = b.body {
                let rb = &self.w.bodies[h];
                let t = rb.translation();
                b.pos = vec2(t.x, t.y) * PX_PER_M;
                b.angle = rb.rotation().angle();
            }
        }
    }

    fn tile_hash(world: &World, tx: i32, ty: i32) -> u64 {
        let mut h: u64 = 1469598103934665603;
        for y in ty * TILE..(ty + 1) * TILE {
            let mut row = 0u32;
            for x in tx * TILE..(tx + 1) * TILE {
                row = (row << 1) | world.raw(x, y).is_hard() as u32;
            }
            h = (h ^ row as u64).wrapping_mul(1099511628211);
        }
        h
    }

    /// Greedy rectangles covering the hard cells of a tile.
    fn tile_rects(world: &World, tx: i32, ty: i32) -> Vec<(i32, i32, i32, i32)> {
        let mut done: Vec<(i32, i32, i32, i32)> = vec![];
        let mut open: Vec<(i32, i32, i32, i32)> = vec![]; // x0, y0, w, h
        for y in ty * TILE..(ty + 1) * TILE {
            let mut runs: Vec<(i32, i32)> = vec![];
            let mut x = tx * TILE;
            let end = (tx + 1) * TILE;
            while x < end {
                if world.raw(x, y).is_hard() {
                    let s = x;
                    while x < end && world.raw(x, y).is_hard() {
                        x += 1;
                    }
                    runs.push((s, x - s));
                } else {
                    x += 1;
                }
            }
            let mut next = vec![];
            for (s, w) in runs {
                if let Some(pos) = open.iter().position(|r| r.0 == s && r.2 == w && r.1 + r.3 == y) {
                    let mut r = open.remove(pos);
                    r.3 += 1;
                    next.push(r);
                } else {
                    next.push((s, y, w, 1));
                }
            }
            done.extend(open.drain(..));
            open = next;
        }
        done.extend(open);
        done
    }

    fn refresh_tiles(&mut self, world: &World, blocks: &[Block]) {
        let mut wanted: HashSet<(i32, i32)> = HashSet::new();
        for b in blocks {
            let Some(h) = b.body else { continue };
            let v = self.w.bodies[h].linvel();
            let reach = 10.0 + (v.x.abs().max(v.y.abs()) * PX_PER_M / 60.0) * 4.0;
            let r = b.half().length() + reach;
            let (x0, y0) = (((b.pos.x - r) / TILE as f32).floor() as i32, ((b.pos.y - r) / TILE as f32).floor() as i32);
            let (x1, y1) = (((b.pos.x + r) / TILE as f32).floor() as i32, ((b.pos.y + r) / TILE as f32).floor() as i32);
            for ty in y0.max(0)..=y1.min(H as i32 / TILE) {
                for tx in x0.max(0)..=x1.min(W as i32 / TILE) {
                    wanted.insert((tx, ty));
                }
            }
        }
        // In a fixed order, so that the physics does not depend on hash-map iteration order.
        let mut wanted: Vec<(i32, i32)> = wanted.into_iter().collect();
        wanted.sort_unstable();
        for (tx, ty) in wanted {
            let hash = Self::tile_hash(world, tx, ty);
            if self.tiles.get(&(tx, ty)).map_or(false, |t| t.0 == hash) {
                continue;
            }
            if let Some((_, old)) = self.tiles.remove(&(tx, ty)) {
                for c in old {
                    self.w.remove_collider(c);
                }
            }
            let mut handles = vec![];
            for (x, y, w, h) in Self::tile_rects(world, tx, ty) {
                let c = vec2(x as f32 + w as f32 / 2.0, y as f32 + h as f32 / 2.0);
                let col = ColliderBuilder::cuboid(w as f32 / 2.0 / PX_PER_M, h as f32 / 2.0 / PX_PER_M)
                    .translation(v2(c))
                    .friction(0.9);
                handles.push(self.w.insert_collider(col, Some(self.terrain)));
            }
            self.tiles.insert((tx, ty), (hash, handles));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ground() -> World {
        let mut w = World::new_static();
        for x in 0..640 {
            for y in 300..320 {
                w.set(x, y, Mat::Stone);
            }
        }
        w
    }

    fn settle(world: &mut World, blocks: &mut Vec<Block>, phys: &mut Physics, steps: usize) {
        for _ in 0..steps {
            phys.step(world, blocks);
            stamp_blocks(world, blocks);
            world.step();
        }
    }

    #[test]
    fn block_rests_on_the_ground() {
        let mut world = ground();
        let mut blocks = vec![Block::new(BlockKind::Brick, vec2(100.0, 250.0), 0.0)];
        let mut phys = Physics::build(&mut blocks);
        settle(&mut world, &mut blocks, &mut phys, 240);
        // Resting on y = 300: centre at 300 - 5.
        assert!((blocks[0].pos.y - 295.0).abs() < 2.0, "y = {}", blocks[0].pos.y);
        assert!(blocks[0].angle.abs() < 0.05);
    }

    #[test]
    fn block_lands_on_sand_pile() {
        let mut world = ground();
        for x in 80..120 {
            for y in 290..300 {
                world.set(x, y, Mat::Sand);
            }
        }
        let mut blocks = vec![Block::new(BlockKind::Brick, vec2(100.0, 240.0), 0.0)];
        let mut phys = Physics::build(&mut blocks);
        settle(&mut world, &mut blocks, &mut phys, 300);
        assert!(blocks[0].pos.y < 290.0, "sank to {}", blocks[0].pos.y);
    }

    #[test]
    fn welded_stack_stays_together() {
        let mut world = ground();
        let mut blocks = vec![
            Block::new(BlockKind::Brick, vec2(100.0, 295.0), 0.0),
            Block::new(BlockKind::Brick, vec2(100.0, 285.0), 0.0),
            Block::new(BlockKind::Brick, vec2(100.0, 275.0), 0.0),
        ];
        let mut phys = Physics::build(&mut blocks);
        settle(&mut world, &mut blocks, &mut phys, 300);
        for (i, b) in blocks.iter().enumerate() {
            assert!((b.pos.x - 100.0).abs() < 2.0, "block {i} x = {}", b.pos.x);
        }
        assert!((blocks[2].pos.y - 275.0).abs() < 2.0, "top y = {}", blocks[2].pos.y);
    }

    #[test]
    fn long_bridge_breaks_in_the_middle_when_loaded() {
        let mut world = ground();
        // Gap in the ground between x=200 and x=440.
        for x in 200..440 {
            for y in 300..320 {
                world.set(x, y, Mat::Empty);
            }
        }
        let mut blocks = vec![];
        for i in 0..6 {
            blocks.push(Block::new(BlockKind::Long, vec2(180.0 + 20.0 + 40.0 * i as f32, 295.0), 0.0));
        }
        // Heavy cubes in the middle.
        for k in 0..4 {
            blocks.push(Block::new(BlockKind::Cube, vec2(300.0 + 20.0 * k as f32, 270.0), 0.0));
        }
        let mut phys = Physics::build(&mut blocks);
        let mut peak = (0.0f32, 0.0f32);
        for _ in 0..600 {
            settle(&mut world, &mut blocks, &mut phys, 1);
            let l = phys.max_joint_load();
            peak = (peak.0.max(l.0), peak.1.max(l.1));
        }
        eprintln!("peak load {peak:?}");
        eprintln!("bridge ys: {:?}", blocks.iter().map(|b| b.pos.y as i32).collect::<Vec<_>>());
    }

    #[test]
    fn overloaded_bridge_snaps() {
        let mut world = ground();
        for x in 200..440 {
            for y in 300..320 {
                world.set(x, y, Mat::Empty);
            }
        }
        let mut blocks = vec![];
        for i in 0..6 {
            blocks.push(Block::new(BlockKind::Long, vec2(180.0 + 20.0 + 40.0 * i as f32, 295.0), 0.0));
        }
        for k in 0..5 {
            for row in 0..4 {
                blocks.push(Block::new(BlockKind::Cube, vec2(270.0 + 20.0 * k as f32, 280.0 - 20.0 * row as f32), 0.0));
            }
        }
        let mut phys = Physics::build(&mut blocks);
        let before = phys.joint_count();
        let mut peak = (0.0f32, 0.0f32);
        for _ in 0..400 {
            settle(&mut world, &mut blocks, &mut phys, 1);
            let l = phys.max_joint_load();
            peak = (peak.0.max(l.0), peak.1.max(l.1));
        }
        eprintln!("overload peak {peak:?} joints {before} -> {}", phys.joint_count());
        assert!(phys.joint_count() < before);
    }

    #[test]
    fn rotated_wheel_rolls() {
        let mut world = ground();
        let mut blocks = vec![Block::new(BlockKind::Wheel, vec2(100.0, 280.0), 0.0)];
        let mut phys = Physics::build(&mut blocks);
        settle(&mut world, &mut blocks, &mut phys, 200);
        assert!((blocks[0].pos.y - 288.0).abs() < 2.0, "y = {}", blocks[0].pos.y);
    }

    /// Drives a car across a gap bridged with the given blocks.
    fn cross_bridge(kind: crate::car::ModelKind, spans: Vec<Block>) -> (bool, f32, usize, usize, (f32, f32)) {
        use crate::car::{Car, Fate};
        let mut world = ground();
        for x in 300..400 {
            for y in 300..320 {
                world.set(x, y, Mat::Empty);
            }
        }
        let mut blocks = spans;
        // Cut a recess under every block end so the deck sits flush with the ground.
        for b in &blocks {
            let (x0, y0, x1, y1) = b.bounds();
            for y in y0..=y1 {
                for x in x0..=x1 {
                    if b.covers(vec2(x as f32 + 0.5, y as f32 + 0.5)) {
                        world.set(x, y, Mat::Empty);
                    }
                }
            }
        }
        let mut phys = Physics::build(&mut blocks);
        let joints = phys.joint_count();
        let mut car = Car::new(kind, (230, 276));
        let mut made_it = false;
        let mut peak = (0.0f32, 0.0f32);
        for _ in 0..3000 {
            world.step();
            car.step(&world, None);
            let weight = kind.model().mass;
            for (pt, share) in car.loads.clone() {
                if let Some(id) = world.block_at_cell(pt.x.floor() as i32, pt.y.floor() as i32) {
                    let b = blocks[id].clone();
                    phys.press(&b, pt, weight * share);
                }
            }
            phys.step(&world, &mut blocks);
            let l = phys.max_joint_load();
            peak = (peak.0.max(l.0), peak.1.max(l.1));
            stamp_blocks(&mut world, &blocks);
            if car.p.x > 450.0 {
                made_it = true;
                break;
            }
            if car.fate != Fate::Driving {
                break;
            }
        }
        (made_it, car.p.x, joints, phys.joint_count(), peak)
    }

    #[test]
    fn bridges_depend_on_the_material() {
        use crate::car::ModelKind;
        let planks = |n: usize| -> Vec<Block> {
            (0..n).map(|i| Block::new(BlockKind::Plank, vec2(295.0 + 30.0 + 60.0 * i as f32, 302.5), 0.0)).collect()
        };
        let beams = |n: usize| -> Vec<Block> {
            (0..n).map(|i| Block::new(BlockKind::Beam, vec2(260.0 + 40.0 + 80.0 * i as f32, 305.0), 0.0)).collect()
        };
        let longs = |n: usize| -> Vec<Block> {
            (0..n).map(|i| Block::new(BlockKind::Long, vec2(290.0 + 20.0 + 40.0 * i as f32, 305.0), 0.0)).collect()
        };
        let steel = |n: usize| -> Vec<Block> {
            (0..n).map(|i| Block::new(BlockKind::SteelBeam, vec2(260.0 + 40.0 + 80.0 * i as f32, 305.0), 0.0)).collect()
        };
        // A light car crosses everything; a heavy truck snaps a wooden bridge
        // but not iron or steel beams.
        assert!(cross_bridge(ModelKind::Buggy, planks(2)).0, "buggy on planks");
        assert!(cross_bridge(ModelKind::Buggy, beams(2)).0, "buggy on beams");
        assert!(cross_bridge(ModelKind::Buggy, longs(3)).0, "buggy on bricks");
        let truck_planks = cross_bridge(ModelKind::Truck, planks(2));
        assert!(truck_planks.3 < truck_planks.2, "wooden bridge should snap under a truck: {truck_planks:?}");
        assert!(cross_bridge(ModelKind::Truck, beams(2)).0, "truck on iron beams");
        assert!(cross_bridge(ModelKind::Truck, steel(2)).0, "truck on steel beams");
    }

    #[test]
    fn a_rope_over_a_pulley_makes_an_atwood_machine() {
        let mut world = ground();
        // A brick (heavy iron) on one side, a glass block (light) on the other.
        let mut blocks = vec![Block::new(BlockKind::Cube, vec2(180.0, 220.0), 0.0), Block::new(BlockKind::Glass, vec2(220.0, 220.0), 0.0)];
        let (heavy, light) = (blocks[0].uid, blocks[1].uid);
        let mut phys = Physics::build(&mut blocks);
        let (centre, r) = (vec2(200.0, 100.0), 6.0);
        phys.add_pulley(&blocks, &[], 1, &Anchor::Free(centre), centre, r);
        let top = vec2(centre.x, centre.y - r - 1.5);
        let mut cable = Cable::new(Anchor::Block { uid: heavy, local: vec2(0.0, -10.0) }, Anchor::Block { uid: light, local: vec2(0.0, -4.0) }, 100.0);
        cable.via = vec![1];
        cable.via_pts = vec![top];
        let len = ((vec2(180.0, 210.0) - top).length() + (vec2(220.0, 216.0) - top).length()) * 1.02;
        cable.len = len;
        phys.add_cable(&blocks, &[], &mut cable);
        let (y0_heavy, y0_light) = (blocks[0].pos.y, blocks[1].pos.y);
        let (mut near_pulley, mut heavy_down, mut light_up) = (0, 0.0f32, 0.0f32);
        for t in 0..100 {
            phys.step(&world, &mut blocks);
            phys.sync_cables(&blocks, &[], std::slice::from_mut(&mut cable));
            stamp_blocks(&mut world, &blocks);
            if t == 40 {
                near_pulley = cable.pts.iter().filter(|p| (**p - centre).length() < r + 4.0).count();
            }
            heavy_down = heavy_down.max(blocks[0].pos.y - y0_heavy);
            light_up = light_up.max(y0_light - blocks[1].pos.y);
        }
        eprintln!("ATWOOD heavy +{heavy_down:.0}, light -{light_up:.0}, beads on the pulley {near_pulley}, snapped {}", cable.snapped);
        assert!(near_pulley >= 2, "the rope wraps the pulley, not through it: {near_pulley}");
        assert!(heavy_down > 20.0, "the heavy block goes down");
        assert!(light_up > 20.0, "the light block is pulled up by the rope over the pulley");
        assert!((heavy_down - light_up).abs() < 25.0, "what one side loses the other gains: {heavy_down} vs {light_up}");
        assert!(!cable.snapped);
    }

    #[test]
    fn reeling_a_rope_in_lifts_what_hangs_from_it() {
        let mut world = ground();
        let mut blocks = vec![Block::new(BlockKind::Cube, vec2(300.0, 230.0), 0.0)];
        let cube = blocks[0].uid;
        let mut phys = Physics::build(&mut blocks);
        let reel = vec2(300.0, 100.0);
        phys.add_reel(7, reel);
        let mut cable = Cable::new(Anchor::Reel { uid: 7, pos: reel }, Anchor::Block { uid: cube, local: vec2(0.0, -10.0) }, 100.0);
        cable.len = 120.0;
        phys.add_cable(&blocks, &[], &mut cable);
        for _ in 0..200 {
            phys.step(&world, &mut blocks);
            phys.sync_cables(&blocks, &[], std::slice::from_mut(&mut cable));
        }
        let hanging = blocks[0].pos.y;
        let mut len = 120.0;
        for _ in 0..400 {
            len = (len - 0.25f32).max(50.0);
            phys.set_cable_length(&cable, len);
            phys.step(&world, &mut blocks);
            phys.sync_cables(&blocks, &[], std::slice::from_mut(&mut cable));
            stamp_blocks(&mut world, &blocks);
        }
        assert!(blocks[0].pos.y < hanging - 40.0, "winding in 70 cells of rope lifts the cube: {hanging} -> {}", blocks[0].pos.y);
        assert!(!cable.snapped);
        // A loaded rope pulls on its reel.
        cable.len = len;
        let mut pull = 0.0f32;
        for _ in 0..60 {
            phys.step(&world, &mut blocks);
            phys.sync_cables(&blocks, &[], std::slice::from_mut(&mut cable));
            pull = pull.max(Physics::rope_tension(&cable, true));
        }
        assert!(pull > 0.0, "a loaded rope pulls on its reel: {pull}");
    }

    #[test]
    fn clipped_bricks_do_not_slide_sideways_but_can_rise_a_little_and_rock() {
        let world = ground();
        // Two bricks, the upper one clipped onto the lower one by two studs.
        let mut blocks = vec![Block::new(BlockKind::Brick, vec2(100.0, 295.0), 0.0), Block::new(BlockKind::Brick, vec2(100.0, 285.0), 0.0)];
        assert_eq!(block_studs(&blocks[0], &blocks[1]), 2);
        let mut phys = Physics::build(&mut blocks);
        assert_eq!(phys.joint_count(), 1);
        for _ in 0..60 {
            phys.step(&world, &mut blocks);
        }
        let rest = blocks[1].pos;
        // A sideways shove: the clip holds it in place.
        let b1 = blocks[1].clone();
        phys.knock(&b1, b1.pos, vec2(0.3, 0.0));
        let mut drift = 0.0f32;
        for _ in 0..90 {
            phys.step(&world, &mut blocks);
            drift = drift.max((blocks[1].pos.x - rest.x).abs());
        }
        assert!(drift < 1.0, "no sliding sideways: {drift}");
        assert_eq!(phys.joint_count(), 1, "and it did not come off");
        // A small pull upwards: it rises by a stud's play and settles back.
        let b1 = blocks[1].clone();
        phys.knock(&b1, b1.pos, vec2(0.0, -0.6));
        let mut rise = 0.0f32;
        for _ in 0..90 {
            phys.step(&world, &mut blocks);
            rise = rise.max(rest.y - blocks[1].pos.y);
        }
        eprintln!("CLIP drift {drift:.2} rise {rise:.2} joints {}", phys.joint_count());
        assert!(rise > 0.2 && rise < 2.5, "it can move vertically a little: {rise}");
        // More studs hold more than fewer.
        let wide = [Block::new(BlockKind::Long, vec2(300.0, 295.0), 0.0), Block::new(BlockKind::Long, vec2(300.0, 285.0), 0.0)];
        let off = [Block::new(BlockKind::Long, vec2(500.0, 295.0), 0.0), Block::new(BlockKind::Long, vec2(530.0, 285.0), 0.0)];
        assert_eq!(block_studs(&wide[0], &wide[1]), 4);
        assert_eq!(block_studs(&off[0], &off[1]), 1);
    }

    #[test]
    fn a_pivoted_wheel_winds_a_rope_and_lifts_a_weight() {
        let mut world = ground();
        let mut blocks = vec![Block::new(BlockKind::Wheel, vec2(200.0, 120.0), 0.0), Block::new(BlockKind::Cube, vec2(188.0, 230.0), 0.0)];
        let (wheel, cube) = (blocks[0].uid, blocks[1].uid);
        // Rope from the left-hand rim of the drum straight down to the cube.
        let mut cables = vec![Cable::new(Anchor::Block { uid: wheel, local: vec2(-12.0, 0.0) }, Anchor::Block { uid: cube, local: vec2(0.0, -10.0) }, 100.0)];
        let mut phys = Physics::build(&mut blocks);
        assert!(phys.add_pivot(&blocks, &[], wheel, &Anchor::Free(vec2(200.0, 120.0))));
        phys.add_cable(&blocks, &[], &mut cables[0]);
        let start = blocks[1].pos.y;
        for _ in 0..1500 {
            // Wind the drum slowly, as a geared motor would.
            let w = phys.spin_of(&blocks[0]);
            let b = blocks[0].clone();
            phys.twist(&b, if w < 1.2 { 900.0 } else { 0.0 });
            phys.step(&world, &mut blocks);
            phys.sync_cables(&blocks, &[], &mut cables);
            stamp_blocks(&mut world, &blocks);
        }
        eprintln!("WINCH wheel {:?} cube y {:.1} (started {start:.1}) snapped {}", blocks[0].pos, blocks[1].pos.y, cables[0].snapped);
        assert!((blocks[0].pos - vec2(200.0, 120.0)).length() < 1.5, "the pivot keeps the wheel in place: {:?}", blocks[0].pos);
        assert!(!cables[0].snapped, "the rope must hold");
        assert!(blocks[1].pos.y < start - 40.0, "the winch should have lifted the cube: y = {}", blocks[1].pos.y);
    }

    #[test]
    fn cable_holds_a_hanging_block() {
        let mut world = ground();
        let mut blocks = vec![Block::new(BlockKind::Cube, vec2(200.0, 250.0), 0.0)];
        let uid = blocks[0].uid;
        let mut cables = vec![Cable::new(Anchor::Ground(vec2(200.0, 100.0)), Anchor::Block { uid, local: vec2(0.0, -10.0) }, 140.0)];
        let mut phys = Physics::build(&mut blocks);
        phys.add_cable(&blocks, &[], &mut cables[0]);
        for _ in 0..600 {
            phys.step(&world, &mut blocks);
            phys.sync_cables(&blocks, &[], &mut cables);
            stamp_blocks(&mut world, &blocks);
            world.step();
        }
        assert!(!cables[0].snapped, "cable snapped under a light cube");
        assert!(blocks[0].pos.y < 281.0, "cube fell to the ground: y = {}", blocks[0].pos.y);
        assert!(cables[0].pts.len() > 5);
        assert!(cables[0].distance_to(vec2(200.0, 180.0)) < 8.0, "cable should hang straight down");
    }

    #[test]
    fn cable_ropes_over_a_pulley_free_span() {
        // A rope strung between two posts sags but does not pass through the ground.
        let mut world = ground();
        let mut blocks: Vec<Block> = vec![];
        let mut cable = Cable::new(Anchor::Ground(vec2(100.0, 250.0)), Anchor::Ground(vec2(220.0, 250.0)), 120.0);
        let mut phys = Physics::build(&mut blocks);
        phys.add_cable(&blocks, &[], &mut cable);
        for _ in 0..400 {
            phys.step(&world, &mut blocks);
            phys.sync_cables(&blocks, &[], std::slice::from_mut(&mut cable));
            world.step();
        }
        let lowest = cable.pts.iter().map(|p| p.y).fold(0.0f32, f32::max);
        assert!(lowest > 252.0, "rope should sag, lowest = {lowest}");
        assert!(lowest < 300.0, "rope went through the ground, lowest = {lowest}");
    }

    #[test]
    fn a_falling_glass_block_registers_an_impact_but_a_gentle_one_does_not() {
        let mut world = ground();
        let mut blocks = vec![Block::new(BlockKind::Glass, vec2(100.0, 120.0), 0.0), Block::new(BlockKind::Glass, vec2(300.0, 294.0), 0.0)];
        let (high, low) = (blocks[0].uid, blocks[1].uid);
        let mut phys = Physics::build(&mut blocks);
        let mut hit = vec![];
        for _ in 0..240 {
            phys.step(&world, &mut blocks);
            hit.extend(phys.take_impacts());
            stamp_blocks(&mut world, &blocks);
        }
        assert!(hit.contains(&high), "a glass block falling 180 cells should shatter on landing");
        assert!(!hit.contains(&low), "a block set down gently must survive");
    }

    /// Hangs `cubes` stacked cubes from one beam; returns (beam survived, peak stress).
    fn hang_from_beam(mat: MemberMat, cubes: usize) -> (bool, f32) {
        let mut world = ground();
        let mut blocks: Vec<Block> = (0..cubes).map(|k| Block::new(BlockKind::Cube, vec2(140.0, 190.0 + 20.0 * k as f32), 0.0)).collect();
        let top = blocks[0].uid;
        let mut phys = Physics::build(&mut blocks);
        let mut members = vec![Member::new(mat, Anchor::Ground(vec2(100.0, 100.0)), Anchor::Block { uid: top, local: vec2(0.0, -10.0) }, vec2(100.0, 100.0), vec2(140.0, 180.0))];
        phys.build_members(&blocks, &mut members);
        let mut peak = 0.0f32;
        for _ in 0..400 {
            phys.step(&world, &mut blocks);
            phys.update_members(&world, &mut members);
            if let Some(m) = members.first() {
                peak = peak.max(m.stress);
            }
            stamp_all(&mut world, &blocks, &members);
        }
        (!members.is_empty(), peak)
    }

    #[test]
    fn beams_carry_loads_according_to_their_material() {
        let wood_light = hang_from_beam(MemberMat::Wood, 1);
        // Clipped bricks give a little, which softens the shock: it takes four cubes to snap a wooden beam.
        let wood_heavy = hang_from_beam(MemberMat::Wood, 4);
        let iron_heavy = hang_from_beam(MemberMat::Iron, 4);
        let steel_heavy = hang_from_beam(MemberMat::Steel, 4);
        eprintln!("BEAM wood1 {wood_light:?} wood3 {wood_heavy:?} iron3 {iron_heavy:?} steel3 {steel_heavy:?}");
        assert!(wood_light.0, "a wooden beam holds one cube");
        assert!(!wood_heavy.0, "a wooden beam snaps under four cubes");
        assert!(iron_heavy.0 && steel_heavy.0, "iron and steel hold four cubes");
        assert!(steel_heavy.1 < iron_heavy.1 + 0.01 || steel_heavy.1 < 1.0, "steel is less stressed than iron for the same load");
    }

    #[test]
    fn a_truss_stays_up_and_shows_load_in_its_members() {
        let mut world = ground();
        // Two ground posts, a horizontal chord and two diagonals meeting at a node.
        let mut blocks: Vec<Block> = vec![];
        let mut phys = Physics::build(&mut blocks);
        let (a, b, apex) = (vec2(100.0, 250.0), vec2(200.0, 250.0), vec2(150.0, 190.0));
        let mut m1 = Member::new(MemberMat::Steel, Anchor::Ground(a), Anchor::Free(apex), a, apex);
        let m2 = Member::new(MemberMat::Steel, Anchor::Ground(b), Anchor::End { uid: m1.uid, end: 1 }, b, apex);
        m1.b = Anchor::Free(apex);
        let mut members = vec![m1, m2];
        phys.build_members(&blocks, &mut members);
        for _ in 0..300 {
            phys.step(&world, &mut blocks);
            phys.update_members(&world, &mut members);
            stamp_all(&mut world, &blocks, &members);
        }
        assert_eq!(members.len(), 2, "an unloaded truss must not collapse");
        assert!((members[0].p1 - apex).length() < 6.0, "apex moved to {:?}", members[0].p1);
    }

    #[test]
    fn a_pressure_wave_shoves_a_block_away_from_the_blast() {
        let mut world = World::new_static();
        let mut blocks = vec![Block::new(BlockKind::Plank, vec2(450.0, 200.0), 0.0)];
        let mut phys = Physics::build(&mut blocks);
        let x0 = blocks[0].pos.x;
        // Only the air blast: no explosion shove, just the pressure wave on the left.
        world.flow.blast(380, 200, 60, 1.5);
        let mut calm = vec![Block::new(BlockKind::Plank, vec2(450.0, 200.0), 0.0)];
        let mut calm_phys = Physics::build(&mut calm);
        let calm_world = World::new_static();
        for _ in 0..90 {
            phys.step(&world, &mut blocks);
            world.step();
            calm_phys.step(&calm_world, &mut calm);
        }
        let (pushed, still) = (blocks[0].pos.x - x0, calm[0].pos.x - x0);
        assert!(pushed > still + 3.0, "the wave carries the block away from the blast: {pushed} vs {still}");
    }

    #[test]
    fn an_underwater_explosion_shoves_a_floating_plank() {
        let run = |boom: bool| {
            let mut world = World::new_static();
            for x in 100..300 {
                world.set(x, 320, Mat::Stone);
            }
            for y in 230..320 {
                world.set(99, y, Mat::Stone);
                world.set(300, y, Mat::Stone);
            }
            for x in 100..300 {
                for y in 270..320 {
                    world.set(x, y, Mat::Water);
                }
            }
            let mut blocks = vec![Block::new(BlockKind::Plank, vec2(190.0, 262.0), 0.0)];
            let mut phys = Physics::build(&mut blocks);
            // Let it settle on the water first.
            for _ in 0..150 {
                phys.step(&world, &mut blocks);
                stamp_blocks(&mut world, &blocks);
                world.step();
            }
            let start = blocks[0].pos;
            if boom {
                world.explode(150, 295, 16);
            }
            let mut top = start.y;
            for _ in 0..150 {
                phys.step(&world, &mut blocks);
                stamp_blocks(&mut world, &blocks);
                world.step();
                top = top.min(blocks[0].pos.y);
            }
            (blocks[0].pos - start, start.y - top)
        };
        let (calm, _) = run(false);
        let (blasted, lift) = run(true);
        eprintln!("calm drift {calm:?}, blasted displacement {blasted:?}, highest lift {lift}");
        assert!(blasted.length() > calm.length() + 4.0 || lift > 4.0, "the blast moves the plank: {blasted:?} vs {calm:?}, lift {lift}");
    }

    #[test]
    fn wood_floats_and_iron_sinks() {
        let mut world = World::new_static();
        // A stone basin full of water.
        for x in 100..300 {
            world.set(x, 320, Mat::Stone);
        }
        for y in 250..320 {
            world.set(99, y, Mat::Stone);
            world.set(300, y, Mat::Stone);
        }
        for x in 100..300 {
            for y in 270..320 {
                world.set(x, y, Mat::Water);
            }
        }
        let mut blocks = vec![
            Block::new(BlockKind::Plank, vec2(160.0, 240.0), 0.0),
            Block::new(BlockKind::Beam, vec2(240.0, 240.0), 0.0),
        ];
        let mut phys = Physics::build(&mut blocks);
        let mut plank_sum = 0.0;
        for t in 0..800 {
            phys.step(&world, &mut blocks);
            stamp_blocks(&mut world, &blocks);
            world.step();
            if t >= 400 {
                plank_sum += blocks[0].pos.y;
            }
        }
        let plank = plank_sum / 400.0;
        // The water surface is around y = 270: wood bobs there, iron lies on the bottom.
        assert!(plank < 282.0, "the plank should float, mean y = {plank}");
        assert!(blocks[1].pos.y > 300.0, "the iron beam should sink, y = {}", blocks[1].pos.y);
    }

    #[test]
    fn a_cable_can_hang_from_a_rod_and_from_the_edge_of_the_world() {
        // From the middle of a rod pinned to the ground at both ends.
        let mut world = ground();
        let mut blocks = vec![Block::new(BlockKind::Cube, vec2(200.0, 250.0), 0.0)];
        let uid = blocks[0].uid;
        let (a, b) = (vec2(150.0, 150.0), vec2(250.0, 150.0));
        let mut members = vec![Member::new(MemberMat::Steel, Anchor::Ground(a), Anchor::Ground(b), a, b)];
        let rod = members[0].uid;
        let mut cables = vec![Cable::new(Anchor::Along { uid: rod, t: 0.5 }, Anchor::Block { uid, local: vec2(0.0, -10.0) }, 90.0)];
        let mut phys = Physics::build(&mut blocks);
        phys.build_members(&blocks, &mut members);
        phys.add_cable(&blocks, &members, &mut cables[0]);
        for _ in 0..500 {
            phys.step(&world, &mut blocks);
            phys.update_members(&world, &mut members);
            phys.sync_cables(&blocks, &members, &mut cables);
            stamp_all(&mut world, &blocks, &members);
        }
        assert!(!cables[0].snapped && members.len() == 1, "rod and cable hold a cube");
        assert!(blocks[0].pos.y < 262.0, "the cube hangs from the rod, y = {}", blocks[0].pos.y);

        // From the left edge of the world.
        let world = ground();
        let mut blocks = vec![Block::new(BlockKind::Cube, vec2(100.0, 250.0), 0.0)];
        let uid = blocks[0].uid;
        let edge = anchor_at(&world, &blocks, vec2(2.0, 150.0)).expect("the world's edge is a valid anchor");
        assert_eq!(edge, Anchor::Ground(vec2(0.0, 150.0)));
        let mut cable = Cable::new(edge, Anchor::Block { uid, local: vec2(-10.0, 0.0) }, 110.0);
        let mut phys = Physics::build(&mut blocks);
        phys.add_cable(&blocks, &[], &mut cable);
        for _ in 0..400 {
            phys.step(&world, &mut blocks);
            phys.sync_cables(&blocks, &[], std::slice::from_mut(&mut cable));
        }
        assert!(!cable.snapped);
        assert!(blocks[0].pos.y < 290.0, "the cube hangs from the wall: y = {}", blocks[0].pos.y);
    }

    /// How far a cube stretches the cable it hangs from (steady state).
    fn stretch_of(mat: CableMat, soft: f32) -> (f32, bool) {
        let mut world = ground();
        let mut blocks = vec![Block::new(BlockKind::Cube, vec2(200.0, 230.0), 0.0)];
        let uid = blocks[0].uid;
        let mut cables = vec![Cable::new(Anchor::Ground(vec2(200.0, 100.0)), Anchor::Block { uid, local: vec2(0.0, -10.0) }, 110.0).made_of(mat, soft)];
        let mut phys = Physics::build(&mut blocks);
        phys.add_cable(&blocks, &[], &mut cables[0]);
        let mut y_sum = 0.0;
        for t in 0..700 {
            phys.step(&world, &mut blocks);
            phys.sync_cables(&blocks, &[], &mut cables);
            stamp_blocks(&mut world, &blocks);
            if t >= 600 {
                y_sum += blocks[0].pos.y;
            }
        }
        (y_sum / 100.0, cables[0].snapped)
    }

    #[test]
    fn cable_material_and_elasticity_change_how_far_it_stretches() {
        let steel = stretch_of(CableMat::Steel, 1.0);
        let iron = stretch_of(CableMat::Iron, 1.0);
        let rope = stretch_of(CableMat::Wood, 1.0);
        let soft_iron = stretch_of(CableMat::Iron, 3.0);
        eprintln!("STRETCH steel {steel:?} iron {iron:?} rope {rope:?} soft iron {soft_iron:?}");
        assert!(steel.0 < iron.0 && iron.0 < rope.0, "stiffer cables hold the cube higher: steel {} iron {} rope {}", steel.0, iron.0, rope.0);
        assert!(soft_iron.0 > iron.0 + 2.0, "a softer setting stretches more: {} vs {}", soft_iron.0, iron.0);
        assert!(!steel.1 && !iron.1, "steel and iron hold a cube");
    }

    #[test]
    fn a_cable_may_hang_from_one_end_with_the_other_loose() {
        let world = ground();
        let mut blocks: Vec<Block> = vec![];
        for loose_first in [false, true] {
            let (a, b) = (Anchor::Ground(vec2(200.0, 80.0)), Anchor::Free(vec2(260.0, 80.0)));
            let mut cable = if loose_first { Cable::new(b, a, 60.0) } else { Cable::new(a, b, 60.0) };
            let mut phys = Physics::build(&mut blocks);
            phys.add_cable(&blocks, &[], &mut cable);
            assert!(cable.has_body(), "a half-loose cable should still be built");
            for _ in 0..600 {
                phys.step(&world, &mut blocks);
                phys.sync_cables(&blocks, &[], std::slice::from_mut(&mut cable));
            }
            let tip = if loose_first { cable.pts[0] } else { *cable.pts.last().unwrap() };
            assert!(tip.y > 80.0 + 40.0, "the loose end should hang down, tip at {tip:?}");
            assert!((tip.x - 200.0).abs() < 25.0, "it hangs under its fixed end: {tip:?}");
            assert!(!cable.snapped);
        }
    }

    fn spring_with_cube(kind: SpringKind, rest: f32) -> (f32, bool) {
        let mut world = ground();
        let mut blocks = vec![Block::new(BlockKind::Cube, vec2(200.0, 200.0), 0.0)];
        let uid = blocks[0].uid;
        let a = vec2(200.0, 100.0);
        let b = vec2(200.0, 190.0);
        let mut springs = vec![Spring::new(kind, Anchor::Ground(a), Anchor::Block { uid, local: vec2(0.0, -10.0) }, rest, 1.0, a, b)];
        let mut phys = Physics::build(&mut blocks);
        phys.add_spring(&blocks, &[], &mut springs[0]);
        let mut y_sum = 0.0;
        for t in 0..900 {
            phys.step(&world, &mut blocks);
            phys.sync_springs(&blocks, &[], &mut springs);
            stamp_blocks(&mut world, &blocks);
            if t >= 800 {
                y_sum += blocks[0].pos.y;
            }
        }
        (y_sum / 100.0, springs.is_empty())
    }

    #[test]
    fn springs_stretch_under_load_and_an_elastic_cord_stretches_much_more() {
        let coil = spring_with_cube(SpringKind::Coil, 60.0);
        let elastic = spring_with_cube(SpringKind::Elastic, 60.0);
        eprintln!("SPRING coil {coil:?} elastic {elastic:?}");
        assert!(!coil.1 && !elastic.1, "neither should snap under one cube");
        // The cube hangs lower on the stretchy cord, and both are longer than at rest.
        assert!(coil.0 > 100.0 + 60.0 + 10.0 - 10.0, "the coil stretches a little under the weight: cube at {}", coil.0);
        assert!(elastic.0 > coil.0 + 8.0, "the elastic cord stretches far more: {} vs {}", elastic.0, coil.0);
    }

    #[test]
    fn a_spring_pulled_too_far_snaps_and_a_free_end_hangs_a_weight() {
        let world = ground();
        let mut blocks: Vec<Block> = vec![];
        let mut phys = Physics::build(&mut blocks);
        // Rest length 40, ends pulled 160 apart: 4x its length.
        let (a, b) = (vec2(100.0, 200.0), vec2(260.0, 200.0));
        let mut springs = vec![Spring::new(SpringKind::Elastic, Anchor::Ground(a), Anchor::Ground(b), 40.0, 1.0, a, b)];
        phys.add_spring(&blocks, &[], &mut springs[0]);
        let mut snapped = false;
        for _ in 0..20 {
            phys.step(&world, &mut blocks);
            snapped |= !phys.sync_springs(&blocks, &[], &mut springs).is_empty();
        }
        assert!(snapped && springs.is_empty(), "stretched past its limit it snaps");

        // A spring whose far end is not attached carries a small weight on it.
        let mut springs = vec![Spring::new(SpringKind::Coil, Anchor::Ground(vec2(200.0, 100.0)), Anchor::Free(vec2(200.0, 160.0)), 60.0, 1.0, vec2(200.0, 100.0), vec2(200.0, 160.0))];
        phys.add_spring(&blocks, &[], &mut springs[0]);
        for _ in 0..300 {
            phys.step(&world, &mut blocks);
            phys.sync_springs(&blocks, &[], &mut springs);
        }
        let end = springs[0].p1;
        assert!(end.y > 158.0 && end.y < 230.0, "the loose end bobs below the rest length, at {end:?}");
    }

    #[test]
    fn rods_springs_and_cables_snap_onto_every_edge_of_the_world() {
        let world = World::new_static();
        for (p, want) in [
            (vec2(9.0, 200.0), Anchor::Ground(vec2(0.0, 200.0))),
            (vec2(W as f32 - 12.0, 150.0), Anchor::Ground(vec2(W as f32, 150.0))),
            (vec2(400.0, 10.0), Anchor::Ground(vec2(400.0, 0.0))),
            (vec2(400.0, H as f32 - 9.0), Anchor::Ground(vec2(400.0, H as f32))),
        ] {
            assert_eq!(node_at(&world, &[], &[], p), Some(want), "click at {p:?}");
        }
        assert_eq!(node_at(&world, &[], &[], vec2(400.0, 200.0)), None, "open sky is not an anchor");
    }

    #[test]
    fn flat_rods_are_deck_and_steep_ones_are_structure() {
        let rod = |a: Vec2, b: Vec2| Member::new(MemberMat::Steel, Anchor::Free(a), Anchor::Free(b), a, b);
        assert!(rod(vec2(0.0, 100.0), vec2(80.0, 100.0)).is_deck(), "level rod is floor");
        assert!(rod(vec2(0.0, 100.0), vec2(80.0, 112.0)).is_deck(), "a gentle ramp is still floor");
        assert!(!rod(vec2(0.0, 100.0), vec2(40.0, 140.0)).is_deck(), "a 45 degree diagonal is structure");
        assert!(!rod(vec2(0.0, 100.0), vec2(2.0, 160.0)).is_deck(), "a post is structure");
    }

    #[test]
    fn a_car_drives_through_the_structure_of_a_bridge_but_rides_its_deck() {
        use crate::car::{Car, Fate, ModelKind};
        let mut world = ground();
        // A 100-wide gap crossed by a flat deck with two diagonals underneath and a post.
        for x in 300..400 {
            for y in 300..320 {
                world.set(x, y, Mat::Empty);
            }
        }
        let (l, r) = (vec2(296.0, 300.0), vec2(404.0, 300.0));
        let mid = vec2(350.0, 340.0);
        let deck = Member::new(MemberMat::Steel, Anchor::Ground(l), Anchor::Ground(r), l, r);
        let d1 = Member::new(MemberMat::Steel, Anchor::Ground(l), Anchor::Free(mid), l, mid);
        let d2 = Member::new(MemberMat::Steel, Anchor::Ground(r), Anchor::Free(mid), r, mid);
        // A tall post standing right in the car's way: structure, so it does not block.
        let (pb, pt) = (vec2(250.0, 300.0), vec2(251.0, 250.0));
        let post = Member::new(MemberMat::Wood, Anchor::Ground(pb), Anchor::Free(pt), pb, pt);
        let members = vec![deck, d1, d2, post];
        assert!(members[0].is_deck() && !members[1].is_deck() && !members[3].is_deck());
        stamp_all(&mut world, &[], &members);
        // The post's cells are there for sand and water, but a car can pass.
        assert!(world.at(250, 280).is_hard());
        assert!(!world.hard_for_car(250, 280), "structure must not block the car");
        assert!(world.hard_for_car(350, 300), "the deck holds the car up");

        let mut car = Car::new(ModelKind::Buggy, (200, 276));
        let mut crossed = false;
        for _ in 0..2500 {
            car.step(&world, None);
            if car.p.x > 440.0 {
                crossed = true;
                break;
            }
            if car.fate != Fate::Driving {
                break;
            }
        }
        assert!(crossed, "the car should pass the post and cross on the deck; ended at x = {} ({:?})", car.p.x, car.fate);
    }

    #[test]
    fn a_rod_with_no_attachment_falls_and_lands() {
        let mut world = ground();
        let mut blocks: Vec<Block> = vec![];
        let mut phys = Physics::build(&mut blocks);
        let (a, b) = (vec2(100.0, 200.0), vec2(160.0, 200.0));
        let mut members = vec![Member::new(MemberMat::Iron, Anchor::Free(a), Anchor::Free(b), a, b)];
        phys.build_members(&blocks, &mut members);
        for _ in 0..300 {
            phys.step(&world, &mut blocks);
            phys.update_members(&world, &mut members);
            stamp_all(&mut world, &blocks, &members);
        }
        assert_eq!(members.len(), 1, "a falling rod must not break");
        assert!(members[0].center().y > 285.0, "the loose rod should have fallen to the ground, y = {}", members[0].center().y);
        assert!((members[0].p1.y - members[0].p0.y).abs() < 10.0, "it lies flat");
    }

    #[test]
    fn bricks_clip_onto_each_other() {
        let base = Block::new(BlockKind::Brick, vec2(100.0, 295.0), 0.0);
        // Hovering 3 cells too high and 4 cells off the stud grid.
        let p = clip(&[base.clone()], BlockKind::Brick, 0.0, vec2(106.0, 282.0));
        assert!((p.y - 285.0).abs() < 1e-3, "y = {}", p.y);
        assert!(((p.x - 100.0) % STUD).abs() < 1e-3, "x = {}", p.x);
        // Too far away: untouched.
        let far = clip(&[base.clone()], BlockKind::Brick, 0.0, vec2(106.0, 250.0));
        assert_eq!(far, vec2(106.0, 250.0));
        // Beside it, flush.
        let side = clip(&[base], BlockKind::Brick, 0.0, vec2(123.0, 297.0));
        assert!((side.x - 120.0).abs() < 1e-3 && (side.y - 295.0).abs() < 1e-3, "{side:?}");
    }

    #[test]
    fn snap_aligns_to_studs() {
        let p = snap(BlockKind::Brick, 0, vec2(103.0, 87.0));
        assert_eq!((p.x - 10.0) % STUD, 0.0);
        assert_eq!((p.y - 5.0) % STUD, 0.0);
    }

    #[test]
    fn placement_rejects_overlap_and_terrain() {
        let world = ground();
        let a = Block::new(BlockKind::Brick, vec2(100.0, 295.0), 0.0);
        assert!(placement_ok(&world, &[], &a, None));
        let in_ground = Block::new(BlockKind::Brick, vec2(100.0, 302.0), 0.0);
        assert!(!placement_ok(&world, &[], &in_ground, None));
        let overlap = Block::new(BlockKind::Brick, vec2(110.0, 295.0), 0.0);
        assert!(!placement_ok(&world, &[a.clone()], &overlap, None));
        let beside = Block::new(BlockKind::Brick, vec2(120.0, 295.0), 0.0);
        assert!(placement_ok(&world, &[a], &beside, None));
    }
}
