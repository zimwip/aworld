//! Saving and loading worlds: grid (run-length encoded), blocks, entry/exit.

use crate::blocks::{Block, BlockKind};
use crate::world::{Mat, World, H, W};
use macroquad::prelude::vec2;
use std::fs;
use std::io::{self, Error, ErrorKind};
use std::path::PathBuf;

const MAGIC: &[u8; 4] = b"AWLD";
const VERSION: u8 = 1;
pub const SLOTS: usize = 3;

pub struct Loaded {
    pub mats: Vec<Mat>,
    pub entry: Option<(i32, i32)>,
    pub exit: Option<(i32, i32)>,
    pub blocks: Vec<Block>,
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

pub fn encode(world: &World, entry: Option<(i32, i32)>, exit: Option<(i32, i32)>, blocks: &[Block]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(MAGIC);
    out.push(VERSION);
    out.extend((W as u16).to_le_bytes());
    out.extend((H as u16).to_le_bytes());
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
    if r.u8()? != VERSION {
        return Err(bad("version de fichier inconnue"));
    }
    if r.u16()? as usize != W || r.u16()? as usize != H {
        return Err(bad("taille de monde différente"));
    }
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
    Ok(Loaded { mats, entry, exit, blocks })
}

pub fn save_slot(slot: usize, world: &World, entry: Option<(i32, i32)>, exit: Option<(i32, i32)>, blocks: &[Block]) -> io::Result<()> {
    fs::create_dir_all(dir())?;
    fs::write(slot_path(slot), encode(world, entry, exit, blocks))
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
        let bytes = encode(&w, Some(e), Some(x), &blocks);
        assert!(bytes.len() < W * H / 4, "RLE should compress: {}", bytes.len());
        let l = decode(&bytes).unwrap();
        assert_eq!(l.entry, Some(e));
        assert_eq!(l.exit, Some(x));
        assert_eq!(l.blocks.len(), 2);
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
        let mut bytes = encode(&w, None, None, &[]);
        bytes.truncate(bytes.len() / 2);
        assert!(decode(&bytes).is_err());
        w.clear();
    }
}
