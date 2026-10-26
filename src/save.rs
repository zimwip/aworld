//! Saving and loading worlds: grid (run-length encoded), blocks, entry/exit.

use crate::pipes::{Driver, Pipe, Valve};
use crate::mech::{Axle, End, Gear, GearSize, Mechanism, Motor, Pivot, Rod, Slider};
use crate::blocks::{Anchor, Block, BlockKind, Cable, CableMat, Member, MemberMat, MemberRole, Spring, SpringKind};
use crate::world::{Biome, Mat, World, DAY_STEPS, H, W};
use macroquad::prelude::{vec2, Vec2};
#[cfg(not(target_arch = "wasm32"))]
use std::fs;
use std::io::{self, Error, ErrorKind};
use std::path::PathBuf;

const MAGIC: &[u8; 4] = b"AWLD";
const VERSION: u8 = 9;

pub struct Loaded {
    pub mats: Vec<Mat>,
    pub entry: Option<(i32, i32)>,
    pub exit: Option<(i32, i32)>,
    pub blocks: Vec<Block>,
    pub cables: Vec<Cable>,
    pub members: Vec<Member>,
    pub springs: Vec<Spring>,
    pub mech: Mechanism,
    pub biome: Biome,
    pub clock: u32,
}

pub fn dir() -> PathBuf {
    PathBuf::from("saves")
}

pub fn slot_path(slot: usize) -> PathBuf {
    dir().join(format!("monde{}.aworld", slot + 1))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn slot_used(slot: usize) -> bool {
    slot_path(slot).exists()
}

// The web build has no files: worlds are kept in memory for as long as the page stays open.
#[cfg(target_arch = "wasm32")]
thread_local! {
    static MEMORY: std::cell::RefCell<[Option<Vec<u8>>; 3]> = const { std::cell::RefCell::new([None, None, None]) };
}

#[cfg(target_arch = "wasm32")]
pub fn slot_used(slot: usize) -> bool {
    MEMORY.with(|m| m.borrow()[slot].is_some())
}

fn put_point(out: &mut Vec<u8>, p: Option<(i32, i32)>) {
    out.push(p.is_some() as u8);
    let (x, y) = p.unwrap_or((0, 0));
    out.extend(x.to_le_bytes());
    out.extend(y.to_le_bytes());
}

pub fn encode(world: &World, entry: Option<(i32, i32)>, exit: Option<(i32, i32)>, blocks: &[Block], cables: &[Cable], members: &[Member], springs: &[Spring], mech: &Mechanism) -> Vec<u8> {
    encode_cells(world.biome, world.clock, |x, y| world.raw(x, y), entry, exit, blocks, cables, members, springs, mech)
}

/// Like `encode`, with the cells read through `raw` (a live world or a captured state).
pub fn encode_cells(biome: Biome, clock: u32, raw: impl Fn(i32, i32) -> Mat, entry: Option<(i32, i32)>, exit: Option<(i32, i32)>, blocks: &[Block], cables: &[Cable], members: &[Member], springs: &[Spring], mech: &Mechanism) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(MAGIC);
    out.push(VERSION);
    out.extend((W as u16).to_le_bytes());
    out.extend((H as u16).to_le_bytes());
    out.push(biome.id());
    out.extend(clock.to_le_bytes());
    put_point(&mut out, entry);
    put_point(&mut out, exit);

    let mut run_mat = raw(0, 0);
    let mut run = 0u32;
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            let m = raw(x, y);
            if m == run_mat {
                run += 1;
            } else {
                out.push(run_mat as u8);
                out.extend(run.to_le_bytes());
                run_mat = m;
                run = 1;
            }
        }
    }
    out.push(run_mat as u8);
    out.extend(run.to_le_bytes());

    out.extend((blocks.len() as u16).to_le_bytes());
    for b in blocks {
        out.push(b.kind.id());
        out.extend(b.pos.x.to_le_bytes());
        out.extend(b.pos.y.to_le_bytes());
        out.extend(b.angle.to_le_bytes());
    }
    let index_of = |uid: u32| blocks.iter().position(|b| b.uid == uid);
    let member_index = |uid: u32| members.iter().position(|m| m.uid == uid);
    let put_anchor = |out: &mut Vec<u8>, a: &Anchor| match a {
        Anchor::Ground(p) => {
            out.push(0);
            out.extend(p.x.to_le_bytes());
            out.extend(p.y.to_le_bytes());
        }
        Anchor::Block { uid, local } => {
            out.push(1);
            out.extend((index_of(*uid).unwrap_or(0) as u16).to_le_bytes());
            out.extend(local.x.to_le_bytes());
            out.extend(local.y.to_le_bytes());
        }
        Anchor::End { uid, end } => {
            out.push(2);
            out.extend((member_index(*uid).unwrap_or(0) as u16).to_le_bytes());
            out.push(*end);
        }
        Anchor::Free(p) => {
            out.push(3);
            out.extend(p.x.to_le_bytes());
            out.extend(p.y.to_le_bytes());
        }
        Anchor::Along { uid, t } => {
            out.push(4);
            out.extend((member_index(*uid).unwrap_or(0) as u16).to_le_bytes());
            out.extend(t.to_le_bytes());
        }
        Anchor::Reel { uid, .. } => {
            out.push(5);
            out.extend(uid.to_le_bytes());
        }
    };
    let saved: Vec<&Cable> = cables
        .iter()
        .filter(|c| {
            [c.a, c.b].iter().all(|a| match a {
                Anchor::Block { uid, .. } => index_of(*uid).is_some(),
                Anchor::End { uid, .. } | Anchor::Along { uid, .. } => member_index(*uid).is_some(),
                _ => true,
            })
        })
        .collect();
    out.extend((saved.len() as u16).to_le_bytes());
    for c in saved {
        put_anchor(&mut out, &c.a);
        put_anchor(&mut out, &c.b);
        out.extend(c.len.to_le_bytes());
        out.push(c.mat.id());
        out.extend(c.soft.to_le_bytes());
        out.extend((c.via.len() as u16).to_le_bytes());
        for u in &c.via {
            out.extend(u.to_le_bytes());
        }
    }
    out.extend((members.len() as u16).to_le_bytes());
    for m in members {
        out.push(m.mat.id());
        out.push(m.role.id());
        put_anchor(&mut out, &m.a);
        put_anchor(&mut out, &m.b);
        for v in [m.p0.x, m.p0.y, m.p1.x, m.p1.y] {
            out.extend(v.to_le_bytes());
        }
    }
    out.extend((springs.len() as u16).to_le_bytes());
    for sp in springs {
        out.push(sp.kind.id());
        put_anchor(&mut out, &sp.a);
        put_anchor(&mut out, &sp.b);
        for v in [sp.rest, sp.soft, sp.p0.x, sp.p0.y, sp.p1.x, sp.p1.y] {
            out.extend(v.to_le_bytes());
        }
    }
    // Mechanisms: gears (with their motor), axles, rails and the rods between them.
    out.extend((mech.gears.len() as u16).to_le_bytes());
    for g in &mech.gears {
        out.extend(g.uid.to_le_bytes());
        out.push(g.size as u8);
        put_anchor(&mut out, &g.mount);
        out.extend(g.angle.to_le_bytes());
        out.push(g.stripped as u8);
        out.extend(g.wheel.and_then(index_of).map_or(0u16, |i| i as u16 + 1).to_le_bytes());
        out.extend(g.reel.unwrap_or(0.0).to_le_bytes());
        match g.motor {
            Some(m) => {
                out.push(1);
                for v in [m.torque, m.free_speed, m.dir, m.reverse_at] {
                    out.extend(v.to_le_bytes());
                }
                out.push(m.auto_reverse as u8);
                out.push(m.steam as u8);
            }
            None => out.push(0),
        }
    }
    out.extend((mech.axles.len() as u16).to_le_bytes());
    for a in &mech.axles {
        for v in [a.uid, a.a, a.b] {
            out.extend(v.to_le_bytes());
        }
    }
    out.extend((mech.sliders.len() as u16).to_le_bytes());
    for sl in &mech.sliders {
        out.extend(sl.uid.to_le_bytes());
        put_anchor(&mut out, &sl.ends.0);
        put_anchor(&mut out, &sl.ends.1);
        out.extend(sl.s.to_le_bytes());
    }
    out.extend((mech.rods.len() as u16).to_le_bytes());
    for r in &mech.rods {
        out.extend(r.uid.to_le_bytes());
        for e in [&r.a, &r.b] {
            match e {
                End::Pin { gear, pin } => {
                    out.push(0);
                    out.extend(gear.to_le_bytes());
                    out.extend(pin.x.to_le_bytes());
                    out.extend(pin.y.to_le_bytes());
                }
                End::Carriage(u) => {
                    out.push(1);
                    out.extend(u.to_le_bytes());
                }
                End::Node(a) => {
                    out.push(2);
                    put_anchor(&mut out, a);
                }
            }
        }
        for v in [r.len, r.strength] {
            out.extend(v.to_le_bytes());
        }
        out.push(r.broken as u8);
    }
    let pivots: Vec<&Pivot> = mech.pivots.iter().filter(|p| index_of(p.wheel).is_some()).collect();
    out.extend((pivots.len() as u16).to_le_bytes());
    for p in pivots {
        out.extend((index_of(p.wheel).unwrap_or(0) as u16).to_le_bytes());
        put_anchor(&mut out, &p.support);
    }
    out.extend((mech.pulleys.len() as u16).to_le_bytes());
    for p in &mech.pulleys {
        out.extend(p.uid.to_le_bytes());
        put_anchor(&mut out, &p.mount);
        out.extend(p.radius.to_le_bytes());
    }
    // Pipes and valves.
    out.extend((mech.plumbing.pipes.len() as u16).to_le_bytes());
    for p in &mech.plumbing.pipes {
        out.extend(p.uid.to_le_bytes());
        for v in [p.a.0, p.a.1, p.b.0, p.b.1] {
            out.extend(v.to_le_bytes());
        }
        out.push(p.fluid as u8);
    }
    out.extend((mech.plumbing.valves.len() as u16).to_le_bytes());
    for v in &mech.plumbing.valves {
        out.extend(v.uid.to_le_bytes());
        out.extend(v.pipe.to_le_bytes());
        for f in [v.t, v.open, v.turns] {
            out.extend(f.to_le_bytes());
        }
        match v.driver {
            None => out.push(0),
            Some(Driver::Gear(u)) => {
                out.push(1);
                out.extend(u.to_le_bytes());
            }
            Some(Driver::Slider(u)) => {
                out.push(2);
                out.extend(u.to_le_bytes());
            }
        }
    }
    out.extend((mech.plumbing.pumps.len() as u16).to_le_bytes());
    for p in &mech.plumbing.pumps {
        out.extend(p.uid.to_le_bytes());
        out.extend(p.pipe.to_le_bytes());
        out.extend(p.t.to_le_bytes());
        out.extend(p.head.to_le_bytes());
        out.push(p.on as u8);
        out.extend(p.driver.unwrap_or(0).to_le_bytes());
    }
    // Version 8: gears that swing a tige (an arm), and lead screws.
    let arms: Vec<(u32, usize, u8)> = mech.gears.iter().filter_map(|g| g.arm.and_then(|(m, e)| Some((g.uid, member_index(m)?, e)))).collect();
    out.extend((arms.len() as u16).to_le_bytes());
    for (gear, m, end) in arms {
        out.extend(gear.to_le_bytes());
        out.extend((m as u16).to_le_bytes());
        out.push(end);
    }
    let screws: Vec<(u32, u32)> = mech.sliders.iter().filter_map(|sl| Some((sl.uid, sl.screw?))).collect();
    out.extend((screws.len() as u16).to_le_bytes());
    for (slider, gear) in screws {
        out.extend(slider.to_le_bytes());
        out.extend(gear.to_le_bytes());
    }
    out
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> io::Result<&[u8]> {
        let end = self.at + n;
        let s = self.data.get(self.at..end).ok_or_else(|| Error::new(ErrorKind::UnexpectedEof, "fichier tronqué"))?;
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> io::Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> io::Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn point(&mut self) -> io::Result<Option<(i32, i32)>> {
        let some = self.u8()? != 0;
        let (x, y) = (self.i32()?, self.i32()?);
        Ok(some.then_some((x, y)))
    }
}

pub fn decode(data: &[u8]) -> io::Result<Loaded> {
    let bad = |m: &str| Error::new(ErrorKind::InvalidData, m.to_string());
    let mut r = Reader { data, at: 0 };
    if r.take(4)? != MAGIC {
        return Err(bad("ce n'est pas un monde AWorld"));
    }
    let version = r.u8()?;
    if !(1..=VERSION).contains(&version) {
        return Err(bad("version de fichier inconnue"));
    }
    if r.u16()? as usize != W || r.u16()? as usize != H {
        return Err(bad("taille de monde différente"));
    }
    let (biome, clock) = if version >= 2 {
        let b = Biome::from_id(r.u8()?);
        (b, r.u32()?)
    } else {
        (Biome::Countryside, DAY_STEPS / 5)
    };
    let entry = r.point()?;
    let exit = r.point()?;
    let mut mats = Vec::with_capacity(W * H);
    while mats.len() < W * H {
        let m = Mat::from_u8(r.u8()?);
        let n = r.u32()? as usize;
        if mats.len() + n > W * H {
            return Err(bad("grille corrompue"));
        }
        mats.extend(std::iter::repeat(m).take(n));
    }
    let count = r.u16()?;
    let mut blocks = vec![];
    for _ in 0..count {
        let kind = BlockKind::from_id(r.u8()?).ok_or_else(|| bad("bloc inconnu"))?;
        let (x, y, a) = (r.f32()?, r.f32()?, r.f32()?);
        blocks.push(Block::new(kind, vec2(x, y), a));
    }
    // Anchors: 0 ground, 1 block, 2 rod end and 4 point along a rod (resolved once
    // the rods exist), 3 free.
    enum Raw {
        Done(Anchor),
        End(usize, u8),
        Along(usize, f32),
    }
    let anchor = |r: &mut Reader| -> io::Result<Raw> {
        Ok(match r.u8()? {
            0 => Raw::Done(Anchor::Ground(vec2(r.f32()?, r.f32()?))),
            1 => {
                let idx = r.u16()? as usize;
                let local = vec2(r.f32()?, r.f32()?);
                let uid = blocks.get(idx).ok_or_else(|| bad("câble sans bloc"))?.uid;
                Raw::Done(Anchor::Block { uid, local })
            }
            2 => Raw::End(r.u16()? as usize, r.u8()?),
            4 => Raw::Along(r.u16()? as usize, r.f32()?),
            5 => Raw::Done(Anchor::Reel { uid: r.u32()?, pos: Vec2::ZERO }),
            _ => Raw::Done(Anchor::Free(vec2(r.f32()?, r.f32()?))),
        })
    };
    let mut raw_cables: Vec<(Raw, Raw, f32, CableMat, f32, Vec<u32>)> = vec![];
    if version >= 2 {
        let count = r.u16()?;
        for _ in 0..count {
            let (a, b) = (anchor(&mut r)?, anchor(&mut r)?);
            let len = r.f32()?;
            let (mat, soft) = if version >= 4 { (CableMat::from_id(r.u8()?), r.f32()?) } else { (CableMat::Iron, 1.0) };
            let mut via = vec![];
            if version >= 6 {
                for _ in 0..r.u16()? {
                    via.push(r.u32()?);
                }
            }
            raw_cables.push((a, b, len, mat, soft, via));
        }
    }
    let mut members: Vec<Member> = vec![];
    let mut pending: Vec<(Raw, Raw)> = vec![];
    if version >= 3 {
        let count = r.u16()?;
        for _ in 0..count {
            let mat = MemberMat::from_id(r.u8()?);
            // Before version 9 a rod's role followed its slope.
            let role = if version >= 9 { Some(MemberRole::from_id(r.u8()?)) } else { None };
            let (a, b) = (anchor(&mut r)?, anchor(&mut r)?);
            let (p0, p1) = (vec2(r.f32()?, r.f32()?), vec2(r.f32()?, r.f32()?));
            let m = Member::new(mat, Anchor::Free(p0), Anchor::Free(p1), p0, p1);
            members.push(match role {
                Some(role) => m.with_role(role),
                None => m,
            });
            pending.push((a, b));
        }
    }
    let mut raw_springs: Vec<(SpringKind, Raw, Raw, [f32; 6])> = vec![];
    if version >= 5 {
        let count = r.u16()?;
        for _ in 0..count {
            let kind = SpringKind::from_id(r.u8()?);
            let (a, b) = (anchor(&mut r)?, anchor(&mut r)?);
            let vals = [r.f32()?, r.f32()?, r.f32()?, r.f32()?, r.f32()?, r.f32()?];
            raw_springs.push((kind, a, b, vals));
        }
    }
    enum RawEnd {
        Pin(u32, Vec2),
        Carriage(u32),
        Node(Raw),
    }
    struct RawGear {
        uid: u32,
        size: GearSize,
        mount: Raw,
        angle: f32,
        stripped: bool,
        wheel: Option<u32>,
        reel: Option<f32>,
        motor: Option<Motor>,
    }
    let mut raw_gears: Vec<RawGear> = vec![];
    let mut axles: Vec<Axle> = vec![];
    let mut raw_sliders: Vec<(u32, Raw, Raw, f32)> = vec![];
    let mut raw_rods: Vec<(u32, [RawEnd; 2], f32, f32, bool)> = vec![];
    let mut raw_pivots: Vec<(usize, Raw)> = vec![];
    let mut plumbing = crate::pipes::Plumbing::default();
    let mut raw_pulleys: Vec<(u32, Raw, f32)> = vec![];
    if version >= 6 {
        for _ in 0..r.u16()? {
            let uid = r.u32()?;
            let size = *GearSize::ALL.get(r.u8()? as usize).ok_or_else(|| bad("engrenage inconnu"))?;
            let mount = anchor(&mut r)?;
            let angle = r.f32()?;
            let stripped = r.u8()? != 0;
            let wheel = match r.u16()? {
                0 => None,
                i => blocks.get(i as usize - 1).map(|b| b.uid),
            };
            let reel = Some(r.f32()?).filter(|v| *v > 0.0);
            let motor = if r.u8()? != 0 {
                let (torque, free_speed, dir, reverse_at) = (r.f32()?, r.f32()?, r.f32()?, r.f32()?);
                {
                    let mut m = Motor::new();
                    m.torque = torque;
                    m.free_speed = free_speed;
                    m.dir = dir;
                    m.reverse_at = reverse_at;
                    m.auto_reverse = r.u8()? != 0;
                    m.steam = version >= 7 && r.u8()? != 0;
                    Some(m)
                }
            } else {
                None
            };
            raw_gears.push(RawGear { uid, size, mount, angle, stripped, wheel, reel, motor });
        }
        for _ in 0..r.u16()? {
            axles.push(Axle { uid: r.u32()?, a: r.u32()?, b: r.u32()? });
        }
        for _ in 0..r.u16()? {
            let uid = r.u32()?;
            let (a, b) = (anchor(&mut r)?, anchor(&mut r)?);
            raw_sliders.push((uid, a, b, r.f32()?));
        }
        for _ in 0..r.u16()? {
            let uid = r.u32()?;
            let mut ends = vec![];
            for _ in 0..2 {
                ends.push(match r.u8()? {
                    0 => RawEnd::Pin(r.u32()?, vec2(r.f32()?, r.f32()?)),
                    1 => RawEnd::Carriage(r.u32()?),
                    _ => RawEnd::Node(anchor(&mut r)?),
                });
            }
            let (len, strength) = (r.f32()?, r.f32()?);
            let broken = r.u8()? != 0;
            let b = ends.pop().unwrap();
            let a = ends.pop().unwrap();
            raw_rods.push((uid, [a, b], len, strength, broken));
        }
        for _ in 0..r.u16()? {
            let w = r.u16()? as usize;
            raw_pivots.push((w, anchor(&mut r)?));
        }
        for _ in 0..r.u16()? {
            let uid = r.u32()?;
            let mount = anchor(&mut r)?;
            raw_pulleys.push((uid, mount, r.f32()?));
        }
        for _ in 0..r.u16()? {
            let uid = r.u32()?;
            let (ax, ay, bx, by) = (r.f32()?, r.f32()?, r.f32()?, r.f32()?);
            let mut p = Pipe::new(uid, (ax, ay), (bx, by));
            p.fluid = Mat::from_u8(r.u8()?);
            plumbing.pipes.push(p);
        }
        for _ in 0..r.u16()? {
            let (uid, pipe) = (r.u32()?, r.u32()?);
            let mut v = Valve::new(uid, pipe, r.f32()?);
            v.open = r.f32()?;
            v.turns = r.f32()?;
            v.driver = match r.u8()? {
                1 => Some(Driver::Gear(r.u32()?)),
                2 => Some(Driver::Slider(r.u32()?)),
                _ => None,
            };
            plumbing.valves.push(v);
        }
        for _ in 0..r.u16()? {
            let (uid, pipe, t, head) = (r.u32()?, r.u32()?, r.f32()?, r.f32()?);
            let mut pm = crate::pipes::Pump::new(uid, pipe, t);
            pm.head = head;
            pm.on = r.u8()? != 0;
            pm.driver = Some(r.u32()?).filter(|u| *u != 0);
            plumbing.pumps.push(pm);
        }
    }
    let mut raw_arms: Vec<(u32, usize, u8)> = vec![];
    let mut raw_screws: Vec<(u32, u32)> = vec![];
    if version >= 8 {
        for _ in 0..r.u16()? {
            raw_arms.push((r.u32()?, r.u16()? as usize, r.u8()?));
        }
        for _ in 0..r.u16()? {
            raw_screws.push((r.u32()?, r.u32()?));
        }
    }
    let uids: Vec<u32> = members.iter().map(|m| m.uid).collect();
    let blocks_uids: Vec<u32> = blocks.iter().map(|b| b.uid).collect();
    let resolve = |raw: Raw, fallback: Vec2| match raw {
        Raw::Done(a) => a,
        Raw::End(i, e) => uids.get(i).map_or(Anchor::Free(fallback), |u| Anchor::End { uid: *u, end: e }),
        Raw::Along(i, t) => uids.get(i).map_or(Anchor::Free(fallback), |u| Anchor::Along { uid: *u, t }),
    };
    for (m, (a, b)) in members.iter_mut().zip(pending) {
        m.a = resolve(a, m.p0);
        m.b = resolve(b, m.p1);
    }
    let mut cables = vec![];
    for (a, b, len, mat, soft, via) in raw_cables {
        // A cable needs one fixed end; the other may be loose. A rod that is missing
        // resolves to a NaN point, and such a cable is dropped.
        let nan = Vec2::splat(f32::NAN);
        let (a, b) = (resolve(a, nan), resolve(b, nan));
        let lost = |x: &Anchor| matches!(x, Anchor::Free(p) if p.x.is_nan());
        if lost(&a) || lost(&b) || (matches!(a, Anchor::Free(_)) && matches!(b, Anchor::Free(_))) {
            continue;
        }
        let mut c = Cable::new(a, b, len / 1.06).made_of(mat, soft);
        c.len = len;
        c.via = via;
        cables.push(c);
    }
    let mut springs = vec![];
    for (kind, a, b, v) in raw_springs {
        let (p0, p1) = (vec2(v[2], v[3]), vec2(v[4], v[5]));
        let (a, b) = (resolve(a, p0), resolve(b, p1));
        springs.push(Spring::new(kind, a, b, v[0], v[1], p0, p1));
    }
    let mut mech = Mechanism::default();
    mech.plumbing = plumbing;
    for g in raw_gears {
        let mut gear = Gear::new(g.uid, g.size, Vec2::ZERO);
        gear.mount = resolve(g.mount, Vec2::ZERO);
        if let Anchor::Free(p) = gear.mount {
            gear.pos = p;
        }
        gear.angle = g.angle;
        gear.stripped = g.stripped;
        gear.wheel = g.wheel;
        gear.reel = g.reel;
        gear.motor = g.motor;
        mech.gears.push(gear);
    }
    mech.axles = axles;
    for (uid, a, b, s) in raw_sliders {
        let (a, b) = (resolve(a, Vec2::ZERO), resolve(b, Vec2::ZERO));
        let mut sl = Slider::new(uid, Vec2::ZERO, vec2(1.0, 0.0));
        sl.ends = (a, b);
        if let (Anchor::Free(p), Anchor::Free(q)) = (a, b) {
            sl.set_ends(p, q);
        }
        sl.s = s;
        mech.sliders.push(sl);
    }
    for (uid, [a, b], len, strength, broken) in raw_rods {
        let end = |e: RawEnd| match e {
            RawEnd::Pin(gear, pin) => End::Pin { gear, pin },
            RawEnd::Carriage(u) => End::Carriage(u),
            RawEnd::Node(raw) => End::Node(resolve(raw, Vec2::ZERO)),
        };
        let mut rod = Rod::between(uid, end(a), end(b), len);
        rod.strength = strength;
        rod.broken = broken;
        mech.rods.push(rod);
    }
    for (uid, mount, radius) in raw_pulleys {
        let mount = resolve(mount, Vec2::ZERO);
        let pos = if let Anchor::Free(p) = mount { p } else { Vec2::ZERO };
        mech.pulleys.push(crate::mech::Pulley { uid, mount, pos, radius });
    }
    for (gear, m, end) in raw_arms {
        if let (Some(g), Some(m)) = (mech.gears.iter_mut().find(|g| g.uid == gear), uids.get(m)) {
            g.arm = Some((*m, end.min(1)));
        }
    }
    for (slider, gear) in raw_screws {
        if let Some(sl) = mech.sliders.iter_mut().find(|s| s.uid == slider) {
            sl.screw = Some(gear);
        }
    }
    mech.prune();
    for (w, a) in raw_pivots {
        if let Some(b) = blocks_uids.get(w) {
            mech.pivots.push(Pivot { wheel: *b, support: resolve(a, Vec2::ZERO) });
        }
    }
    let top = mech.gears.iter().map(|g| g.uid).chain(mech.plumbing.pipes.iter().map(|p| p.uid)).chain(mech.plumbing.valves.iter().map(|v| v.uid)).chain(mech.plumbing.pumps.iter().map(|p| p.uid)).chain(mech.pulleys.iter().map(|p| p.uid)).chain(mech.axles.iter().map(|a| a.uid)).chain(mech.sliders.iter().map(|s| s.uid)).chain(mech.rods.iter().map(|r| r.uid)).max().unwrap_or(0);
    crate::mech::bump_uids(top);
    Ok(Loaded { mats, entry, exit, blocks, cables, members, springs, mech, biome, clock })
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save_slot(slot: usize, bytes: &[u8]) -> io::Result<()> {
    fs::create_dir_all(dir())?;
    fs::write(slot_path(slot), bytes)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load_slot(slot: usize) -> io::Result<Loaded> {
    decode(&fs::read(slot_path(slot))?)
}

#[cfg(target_arch = "wasm32")]
pub fn save_slot(slot: usize, bytes: &[u8]) -> io::Result<()> {
    MEMORY.with(|m| m.borrow_mut()[slot] = Some(bytes.to_vec()));
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn load_slot(slot: usize) -> io::Result<Loaded> {
    let bytes = MEMORY.with(|m| m.borrow()[slot].clone()).ok_or_else(|| Error::new(ErrorKind::NotFound, "emplacement vide"))?;
    decode(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_everything() {
        let mut w = World::new();
        let (e, x) = w.generate(5);
        let blocks = vec![
            Block::new(BlockKind::Brick, vec2(100.0, 50.0), 0.0),
            Block::new(BlockKind::Wheel, vec2(300.5, 80.0), 1.5),
        ];
        let uid = blocks[0].uid;
        let cables = vec![Cable::new(Anchor::Ground(vec2(5.0, 6.0)), Anchor::Block { uid, local: vec2(1.0, 2.0) }, 90.0).made_of(CableMat::Steel, 1.7)];
        w.set_season(crate::world::Season::Autumn);
        let m1 = Member::new(MemberMat::Iron, Anchor::Ground(vec2(10.0, 10.0)), Anchor::Free(vec2(60.0, 10.0)), vec2(10.0, 10.0), vec2(60.0, 10.0));
        let m2 = Member::new(MemberMat::Wood, Anchor::End { uid: m1.uid, end: 1 }, Anchor::Free(vec2(90.0, 40.0)), vec2(60.0, 10.0), vec2(90.0, 40.0)).with_role(MemberRole::Deck);
        let members = vec![m1, m2];
        let sp = Spring::new(SpringKind::Elastic, Anchor::Ground(vec2(3.0, 4.0)), Anchor::End { uid: members[1].uid, end: 0 }, 40.0, 1.5, vec2(3.0, 4.0), vec2(9.0, 9.0));
        let bytes = encode(&w, Some(e), Some(x), &blocks, &cables, &members, &[sp], &Mechanism::default());
        assert!(bytes.len() < W * H / 4, "RLE should compress: {}", bytes.len());
        let l = decode(&bytes).unwrap();
        assert_eq!(l.entry, Some(e));
        assert_eq!(l.exit, Some(x));
        assert_eq!(l.blocks.len(), 2);
        assert_eq!(l.cables.len(), 1);
        assert_eq!(l.cables[0].mat, CableMat::Steel);
        assert!((l.cables[0].soft - 1.7).abs() < 1e-6);
        assert_eq!(l.members.len(), 2);
        assert_eq!(l.springs.len(), 1);
        assert_eq!(l.springs[0].kind, SpringKind::Elastic);
        assert!((l.springs[0].rest - 40.0).abs() < 1e-6 && (l.springs[0].soft - 1.5).abs() < 1e-6);
        assert_eq!(l.springs[0].b, Anchor::End { uid: l.members[1].uid, end: 0 });
        assert_eq!(l.members[1].mat, MemberMat::Wood);
        assert_eq!(l.members[1].role, MemberRole::Deck, "a steep deck stays a deck");
        assert_eq!(l.members[0].role, MemberRole::Deck);
        assert_eq!(l.members[1].a, Anchor::End { uid: l.members[0].uid, end: 1 }, "beam link survives");
        assert_eq!(l.members[0].a, Anchor::Ground(vec2(10.0, 10.0)));
        assert_eq!(l.biome, w.biome);
        assert_eq!(l.clock, w.clock);
        match l.cables[0].b {
            Anchor::Block { uid, local } => {
                assert_eq!(uid, l.blocks[0].uid);
                assert_eq!(local, vec2(1.0, 2.0));
            }
            _ => panic!("anchor lost"),
        }
        assert_eq!(l.blocks[1].kind, BlockKind::Wheel);
        assert!((l.blocks[1].angle - 1.5).abs() < 1e-6);
        let mut w2 = World::new();
        w2.load_mats(&l.mats);
        for i in (0..W * H).step_by(997) {
            assert_eq!(w.raw((i % W) as i32, (i / W) as i32), w2.raw((i % W) as i32, (i / W) as i32));
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode(b"nope").is_err());
        assert!(decode(&[]).is_err());
        let mut w = World::new();
        let mut bytes = encode(&w, None, None, &[], &[], &[], &[], &Mechanism::default());
        bytes.truncate(bytes.len() / 2);
        assert!(decode(&bytes).is_err());
        w.clear();
    }
}
