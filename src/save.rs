//! Saving and loading worlds: grid (run-length encoded), blocks, entry/exit.

use crate::blocks::{Anchor, Block, BlockKind, Cable, Member, MemberMat};
use crate::world::{Biome, Mat, World, DAY_STEPS, H, W};
use macroquad::prelude::{vec2, Vec2};
use std::fs;
use std::io::{self, Error, ErrorKind};
use std::path::PathBuf;

const MAGIC: &[u8; 4] = b"AWLD";
const VERSION: u8 = 3;

pub struct Loaded {
    pub mats: Vec<Mat>,
    pub entry: Option<(i32, i32)>,
    pub exit: Option<(i32, i32)>,
    pub blocks: Vec<Block>,
    pub cables: Vec<Cable>,
    pub members: Vec<Member>,
    pub biome: Biome,
    pub clock: u32,
}

pub fn dir() -> PathBuf {
    PathBuf::from("saves")
}

pub fn slot_path(slot: usize) -> PathBuf {
    dir().join(format!("monde{}.aworld", slot + 1))
}

pub fn slot_used(slot: usize) -> bool {
    slot_path(slot).exists()
}

fn put_point(out: &mut Vec<u8>, p: Option<(i32, i32)>) {
    out.push(p.is_some() as u8);
    let (x, y) = p.unwrap_or((0, 0));
    out.extend(x.to_le_bytes());
    out.extend(y.to_le_bytes());
}

pub fn encode(world: &World, entry: Option<(i32, i32)>, exit: Option<(i32, i32)>, blocks: &[Block], cables: &[Cable], members: &[Member]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(MAGIC);
    out.push(VERSION);
    out.extend((W as u16).to_le_bytes());
    out.extend((H as u16).to_le_bytes());
    out.push(world.biome.id());
    out.extend(world.clock.to_le_bytes());
    put_point(&mut out, entry);
    put_point(&mut out, exit);

    let mut run_mat = world.raw(0, 0);
    let mut run = 0u32;
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            let m = world.raw(x, y);
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
    };
    let saved: Vec<&Cable> = cables
        .iter()
        .filter(|c| [c.a, c.b].iter().all(|a| !matches!(a, Anchor::Block { uid, .. } if index_of(*uid).is_none())))
        .collect();
    out.extend((saved.len() as u16).to_le_bytes());
    for c in saved {
        put_anchor(&mut out, &c.a);
        put_anchor(&mut out, &c.b);
        out.extend(c.len.to_le_bytes());
    }
    out.extend((members.len() as u16).to_le_bytes());
    for m in members {
        out.push(m.mat.id());
        put_anchor(&mut out, &m.a);
        put_anchor(&mut out, &m.b);
        for v in [m.p0.x, m.p0.y, m.p1.x, m.p1.y] {
            out.extend(v.to_le_bytes());
        }
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
    if !(1..=3).contains(&version) {
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
    // Anchors: 0 ground, 1 block, 2 beam end (resolved later), 3 free.
    enum Raw {
        Done(Anchor),
        End(usize, u8),
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
            _ => Raw::Done(Anchor::Free(vec2(r.f32()?, r.f32()?))),
        })
    };
    let mut cables = vec![];
    if version >= 2 {
        let count = r.u16()?;
        for _ in 0..count {
            let (a, b) = (anchor(&mut r)?, anchor(&mut r)?);
            let len = r.f32()?;
            let (Raw::Done(a), Raw::Done(b)) = (a, b) else { return Err(bad("câble mal formé")) };
            let mut c = Cable::new(a, b, len / 1.06);
            c.len = len;
            cables.push(c);
        }
    }
    let mut members: Vec<Member> = vec![];
    if version >= 3 {
        let count = r.u16()?;
        let mut pending: Vec<(Raw, Raw)> = vec![];
        for _ in 0..count {
            let mat = MemberMat::from_id(r.u8()?);
            let (a, b) = (anchor(&mut r)?, anchor(&mut r)?);
            let (p0, p1) = (vec2(r.f32()?, r.f32()?), vec2(r.f32()?, r.f32()?));
            members.push(Member::new(mat, Anchor::Free(p0), Anchor::Free(p1), p0, p1));
            pending.push((a, b));
        }
        let uids: Vec<u32> = members.iter().map(|m| m.uid).collect();
        let resolve = |raw: Raw, fallback: Vec2| match raw {
            Raw::Done(a) => a,
            Raw::End(i, e) => uids.get(i).map_or(Anchor::Free(fallback), |u| Anchor::End { uid: *u, end: e }),
        };
        for (m, (a, b)) in members.iter_mut().zip(pending) {
            m.a = resolve(a, m.p0);
            m.b = resolve(b, m.p1);
        }
    }
    Ok(Loaded { mats, entry, exit, blocks, cables, members, biome, clock })
}

pub fn save_slot(slot: usize, world: &World, entry: Option<(i32, i32)>, exit: Option<(i32, i32)>, blocks: &[Block], cables: &[Cable], members: &[Member]) -> io::Result<()> {
    fs::create_dir_all(dir())?;
    fs::write(slot_path(slot), encode(world, entry, exit, blocks, cables, members))
}

pub fn load_slot(slot: usize) -> io::Result<Loaded> {
    decode(&fs::read(slot_path(slot))?)
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
        let cables = vec![Cable::new(Anchor::Ground(vec2(5.0, 6.0)), Anchor::Block { uid, local: vec2(1.0, 2.0) }, 90.0)];
        w.set_season(crate::world::Season::Autumn);
        let m1 = Member::new(MemberMat::Iron, Anchor::Ground(vec2(10.0, 10.0)), Anchor::Free(vec2(60.0, 10.0)), vec2(10.0, 10.0), vec2(60.0, 10.0));
        let m2 = Member::new(MemberMat::Wood, Anchor::End { uid: m1.uid, end: 1 }, Anchor::Free(vec2(90.0, 40.0)), vec2(60.0, 10.0), vec2(90.0, 40.0));
        let members = vec![m1, m2];
        let bytes = encode(&w, Some(e), Some(x), &blocks, &cables, &members);
        assert!(bytes.len() < W * H / 4, "RLE should compress: {}", bytes.len());
        let l = decode(&bytes).unwrap();
        assert_eq!(l.entry, Some(e));
        assert_eq!(l.exit, Some(x));
        assert_eq!(l.blocks.len(), 2);
        assert_eq!(l.cables.len(), 1);
        assert_eq!(l.members.len(), 2);
        assert_eq!(l.members[1].mat, MemberMat::Wood);
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
        let mut bytes = encode(&w, None, None, &[], &[], &[]);
        bytes.truncate(bytes.len() / 2);
        assert!(decode(&bytes).is_err());
        w.clear();
    }
}
