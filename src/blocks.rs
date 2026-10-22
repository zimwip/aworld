//! Lego-like rigid blocks. They live outside the sand grid: while building
//! they are plain shapes snapped to a stud grid; when the simulation runs
//! they become rapier bodies, welded to the blocks they touch.

use crate::world::{Mat, World, H, W};
use macroquad::prelude::{vec2, Vec2};
use rapier2d::prelude::*;
use std::collections::{HashMap, HashSet};

/// Size of one stud in cells; blocks snap to this grid.
pub const STUD: f32 = 12.0;
/// Cells per physics meter.
const PX_PER_M: f32 = 20.0;
const TILE: i32 = 16;
/// Weld breaking limits (per step): force and torque impulses.
const BREAK_FORCE: f32 = 9.0;
const BREAK_TORQUE: f32 = 14.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockKind {
    Brick,
    Long,
    Plank,
    Beam,
    Cube,
    Wheel,
}

pub struct Spec {
    pub name: &'static str,
    pub w: f32,
    pub h: f32,
    pub mat: Mat,
    pub round: bool,
    pub density: f32,
    pub color: (u8, u8, u8),
}

impl BlockKind {
    pub const ALL: [BlockKind; 6] =
        [BlockKind::Brick, BlockKind::Long, BlockKind::Plank, BlockKind::Beam, BlockKind::Cube, BlockKind::Wheel];

    pub fn id(self) -> u8 {
        Self::ALL.iter().position(|k| *k == self).unwrap() as u8
    }

    pub fn from_id(id: u8) -> Option<BlockKind> {
        Self::ALL.get(id as usize).copied()
    }

    pub fn spec(self) -> Spec {
        match self {
            BlockKind::Brick => Spec { name: "Brique", w: 24.0, h: 12.0, mat: Mat::Brick, round: false, density: 1.0, color: (205, 62, 52) },
            BlockKind::Long => Spec { name: "Longue", w: 48.0, h: 12.0, mat: Mat::Brick, round: false, density: 1.0, color: (58, 112, 214) },
            BlockKind::Plank => Spec { name: "Planche", w: 72.0, h: 6.0, mat: Mat::Wood, round: false, density: 0.6, color: (176, 124, 72) },
            BlockKind::Beam => Spec { name: "Poutre", w: 96.0, h: 12.0, mat: Mat::Iron, round: false, density: 1.8, color: (150, 160, 178) },
            BlockKind::Cube => Spec { name: "Cube", w: 24.0, h: 24.0, mat: Mat::Stone, round: false, density: 1.4, color: (120, 120, 134) },
            BlockKind::Wheel => Spec { name: "Roue", w: 28.0, h: 28.0, mat: Mat::Iron, round: true, density: 1.2, color: (60, 62, 74) },
        }
    }
}

fn rot(a: f32, v: Vec2) -> Vec2 {
    let (s, c) = a.sin_cos();
    vec2(v.x * c - v.y * s, v.x * s + v.y * c)
}

#[derive(Clone)]
pub struct Block {
    pub kind: BlockKind,
    pub pos: Vec2,
    pub angle: f32,
    /// Accumulated fire damage (wood only).
    pub heat: u16,
    pub body: Option<RigidBodyHandle>,
}

impl Block {
    pub fn new(kind: BlockKind, pos: Vec2, angle: f32) -> Self {
        Block { kind, pos, angle, heat: 0, body: None }
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

/// Stamps the blocks into the world's solid mask.
pub fn stamp_blocks(world: &mut World, blocks: &[Block]) {
    world.begin_blocks(blocks.iter().map(|b| b.spec().mat).collect());
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
    world.end_blocks();
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
    blocks.iter().enumerate().all(|(i, b)| Some(i) == ignore || cand.gap(b) > -1.0)
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

pub fn block_at(blocks: &[Block], p: Vec2) -> Option<usize> {
    blocks.iter().rposition(|b| b.covers(p))
}

// ---------------------------------------------------------------------------

pub struct Physics {
    w: PhysicsWorld,
    terrain: RigidBodyHandle,
    tiles: HashMap<(i32, i32), (u64, Vec<ColliderHandle>)>,
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
        Physics { w, terrain, tiles: HashMap::new() }
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

    fn weld(&mut self, a: &Block, b: &Block) {
        let (Some(ha), Some(hb)) = (a.body, b.body) else { return };
        let mid = (a.pos + b.pos) * 0.5;
        let la = rot(-a.angle, mid - a.pos) / PX_PER_M;
        let lb = rot(-b.angle, mid - b.pos) / PX_PER_M;
        let joint = FixedJointBuilder::new()
            .local_frame1(Pose::new(Vector::new(la.x, la.y), 0.0))
            .local_frame2(Pose::new(Vector::new(lb.x, lb.y), a.angle - b.angle))
            .contacts_enabled(false);
        self.w.insert_impulse_joint(ha, hb, joint);
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

    /// Largest (force, torque) load currently carried by a weld.
    pub fn max_joint_load(&self) -> (f32, f32) {
        self.w.impulse_joints().fold((0.0, 0.0), |(f, t): (f32, f32), (_, j)| {
            (f.max((j.impulses.x * j.impulses.x + j.impulses.y * j.impulses.y).sqrt()), t.max(j.impulses.z.abs()))
        })
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

    pub fn step(&mut self, world: &World, blocks: &mut [Block]) {
        self.refresh_tiles(world, blocks);
        self.w.step();

        let broken: Vec<ImpulseJointHandle> = self
            .w
            .impulse_joints()
            .filter(|(_, j)| {
                let f = (j.impulses.x * j.impulses.x + j.impulses.y * j.impulses.y).sqrt();
                f > BREAK_FORCE || j.impulses.z.abs() > BREAK_TORQUE
            })
            .map(|(h, _)| h)
            .collect();
        for h in broken {
            self.w.remove_impulse_joint(h);
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
        let mut w = World::new();
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
        // Resting on y = 300: centre at 300 - 6.
        assert!((blocks[0].pos.y - 294.0).abs() < 2.0, "y = {}", blocks[0].pos.y);
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
            Block::new(BlockKind::Brick, vec2(100.0, 294.0), 0.0),
            Block::new(BlockKind::Brick, vec2(100.0, 282.0), 0.0),
            Block::new(BlockKind::Brick, vec2(100.0, 270.0), 0.0),
        ];
        let mut phys = Physics::build(&mut blocks);
        settle(&mut world, &mut blocks, &mut phys, 300);
        for (i, b) in blocks.iter().enumerate() {
            assert!((b.pos.x - 100.0).abs() < 2.0, "block {i} x = {}", b.pos.x);
        }
        assert!((blocks[2].pos.y - 270.0).abs() < 2.0, "top y = {}", blocks[2].pos.y);
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
            blocks.push(Block::new(BlockKind::Long, vec2(180.0 + 24.0 + 48.0 * i as f32, 294.0), 0.0));
        }
        // Heavy cubes in the middle.
        for k in 0..4 {
            blocks.push(Block::new(BlockKind::Cube, vec2(300.0 + 24.0 * k as f32, 270.0), 0.0));
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
    fn rotated_wheel_rolls() {
        let mut world = ground();
        let mut blocks = vec![Block::new(BlockKind::Wheel, vec2(100.0, 280.0), 0.0)];
        let mut phys = Physics::build(&mut blocks);
        settle(&mut world, &mut blocks, &mut phys, 200);
        assert!((blocks[0].pos.y - 286.0).abs() < 2.0, "y = {}", blocks[0].pos.y);
    }

    #[test]
    fn snap_aligns_to_studs() {
        let p = snap(BlockKind::Brick, 0, vec2(103.0, 87.0));
        assert_eq!((p.x - 12.0) % STUD, 0.0);
        assert_eq!((p.y - 6.0) % STUD, 0.0);
    }

    #[test]
    fn placement_rejects_overlap_and_terrain() {
        let world = ground();
        let a = Block::new(BlockKind::Brick, vec2(100.0, 294.0), 0.0);
        assert!(placement_ok(&world, &[], &a, None));
        let in_ground = Block::new(BlockKind::Brick, vec2(100.0, 302.0), 0.0);
        assert!(!placement_ok(&world, &[], &in_ground, None));
        let overlap = Block::new(BlockKind::Brick, vec2(110.0, 294.0), 0.0);
        assert!(!placement_ok(&world, &[a.clone()], &overlap, None));
        let beside = Block::new(BlockKind::Brick, vec2(124.0, 294.0), 0.0);
        assert!(placement_ok(&world, &[a], &beside, None));
    }
}
