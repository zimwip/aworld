//! Falling-sand world: one material per cell, simulated cell by cell.
//! Rigid blocks live outside the grid; they are stamped into a mask each step
//! so particles and the car treat them as solid.

pub const W: usize = 640;
pub const H: usize = 360;
/// Resolution factor relative to the 320x180 design grid.
pub const K: i32 = 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Mat {
    Empty,
    Sand,
    Water,
    Fire,
    Steam,
    Iron,
    Wood,
    Stone,
    Brick,
    Rust,
    Ash,
    Ember,
    Lava,
    Ice,
    Gunpowder,
}

impl Mat {
    pub const ALL: [Mat; 15] = [
        Mat::Empty, Mat::Sand, Mat::Water, Mat::Fire, Mat::Steam, Mat::Iron, Mat::Wood, Mat::Stone,
        Mat::Brick, Mat::Rust, Mat::Ash, Mat::Ember, Mat::Lava, Mat::Ice, Mat::Gunpowder,
    ];

    pub fn from_u8(v: u8) -> Mat {
        Mat::ALL.get(v as usize).copied().unwrap_or(Mat::Empty)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Empty,
    Static,
    Powder,
    Liquid,
    Gas,
    Flame,
}

impl Mat {
    pub fn kind(self) -> Kind {
        use Mat::*;
        match self {
            Empty => Kind::Empty,
            Sand | Rust | Ash | Gunpowder => Kind::Powder,
            Water | Lava => Kind::Liquid,
            Steam => Kind::Gas,
            Fire => Kind::Flame,
            Iron | Wood | Stone | Brick | Ember | Ice => Kind::Static,
        }
    }

    /// Blocks the car and rigid blocks.
    pub fn is_hard(self) -> bool {
        matches!(self.kind(), Kind::Static | Kind::Powder)
    }

    fn hot(self) -> bool {
        matches!(self, Mat::Fire | Mat::Ember | Mat::Lava)
    }
}

#[derive(Clone, Copy)]
pub struct Cell {
    pub m: Mat,
    pub life: u8,
    stamp: u8,
}

const EMPTY: Cell = Cell { m: Mat::Empty, life: 0, stamp: 0 };

pub struct World {
    pub cells: Vec<Cell>,
    frame: u8,
    rng: u32,
    seen: Vec<bool>,
    stack: Vec<usize>,
    mask: Vec<u16>,
    mask_prev: Vec<u16>,
    block_mat: Vec<Mat>,
    /// Block ids (index into the block list) touched by flames this step.
    pub touched: Vec<u16>,
    /// Explosions this step: (x, y, radius).
    pub blasts: Vec<(i32, i32, i32)>,
}

impl World {
    pub fn new() -> Self {
        World {
            cells: vec![EMPTY; W * H],
            frame: 0,
            rng: 0x1234_5678,
            seen: vec![false; W * H],
            stack: Vec::new(),
            mask: vec![0; W * H],
            mask_prev: vec![0; W * H],
            block_mat: Vec::new(),
            touched: Vec::new(),
            blasts: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.cells.fill(EMPTY);
    }

    pub fn snapshot(&self) -> Vec<Cell> {
        self.cells.clone()
    }

    pub fn restore(&mut self, snap: &[Cell]) {
        self.cells.copy_from_slice(snap);
    }

    pub fn rand(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    fn chance(&mut self, n: u32) -> bool {
        self.rand() % n == 0
    }

    fn dir(&mut self) -> i32 {
        if self.rand() & 1 == 0 { -1 } else { 1 }
    }

    pub fn inb(x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as usize) < W && (y as usize) < H
    }

    fn idx(x: i32, y: i32) -> usize {
        y as usize * W + x as usize
    }

    /// Material at a cell, blocks included. The ceiling is stone; sides and
    /// bottom are a void where anything that falls in disappears.
    pub fn at(&self, x: i32, y: i32) -> Mat {
        if Self::inb(x, y) {
            let i = Self::idx(x, y);
            let b = self.mask[i];
            if b != 0 {
                return self.block_mat[b as usize - 1];
            }
            self.cells[i].m
        } else if y < 0 {
            Mat::Stone
        } else {
            Mat::Empty
        }
    }

    /// Material of the grid itself, ignoring rigid blocks.
    pub fn raw(&self, x: i32, y: i32) -> Mat {
        if Self::inb(x, y) { self.cells[Self::idx(x, y)].m } else { Mat::Empty }
    }

    /// Obstacle test for the car; the world has no floor below it.
    pub fn hard_for_car(&self, x: i32, y: i32) -> bool {
        if y >= H as i32 {
            return false;
        }
        if x < 0 || x >= W as i32 {
            return true;
        }
        self.at(x, y).is_hard()
    }

    pub fn set(&mut self, x: i32, y: i32, m: Mat) {
        if !Self::inb(x, y) {
            return;
        }
        let life = match m {
            Mat::Fire => 20 + (self.rand() % 40) as u8,
            Mat::Ember => 100 + (self.rand() % 100) as u8,
            _ => 0,
        };
        let frame = self.frame;
        self.cells[Self::idx(x, y)] = Cell { m, life, stamp: frame };
    }

    fn swap(&mut self, a: (i32, i32), b: (i32, i32)) {
        if !Self::inb(b.0, b.1) {
            self.cells[Self::idx(a.0, a.1)] = EMPTY;
            return;
        }
        let (ia, ib) = (Self::idx(a.0, a.1), Self::idx(b.0, b.1));
        self.cells.swap(ia, ib);
        self.cells[ia].stamp = self.frame;
        self.cells[ib].stamp = self.frame;
    }

    // ----- rigid block mask -------------------------------------------------

    pub fn begin_blocks(&mut self, mats: Vec<Mat>) {
        std::mem::swap(&mut self.mask, &mut self.mask_prev);
        self.mask.fill(0);
        self.block_mat = mats;
    }

    pub fn mark_block(&mut self, x: i32, y: i32, id: usize) {
        if Self::inb(x, y) {
            self.mask[Self::idx(x, y)] = id as u16 + 1;
        }
    }

    /// Pushes loose matter out of cells newly covered by a block.
    pub fn end_blocks(&mut self) {
        for i in 0..W * H {
            if self.mask[i] == 0 || self.mask_prev[i] != 0 {
                continue;
            }
            let c = self.cells[i];
            if !matches!(c.m.kind(), Kind::Powder | Kind::Liquid | Kind::Gas | Kind::Flame) {
                continue;
            }
            let (x, y) = ((i % W) as i32, (i / W) as i32);
            self.cells[i] = EMPTY;
            for dy in 1..=30 {
                let j = (y - dy) as usize * W + x as usize;
                if y - dy < 0 {
                    break;
                }
                if self.mask[j] == 0 && self.cells[j].m == Mat::Empty {
                    self.cells[j] = Cell { stamp: self.frame, ..c };
                    break;
                }
            }
        }
    }

    // ----- painting ---------------------------------------------------------

    /// Paint a disc. Loose materials only fill empty cells, solids overwrite.
    pub fn paint(&mut self, cx: i32, cy: i32, r: i32, m: Mat) {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy > r * r + r / 2 {
                    continue;
                }
                let (x, y) = (cx + dx, cy + dy);
                if !Self::inb(x, y) || self.mask[Self::idx(x, y)] != 0 {
                    continue;
                }
                match m.kind() {
                    Kind::Empty => self.set(x, y, Mat::Empty),
                    Kind::Static => self.set(x, y, m),
                    Kind::Gas | Kind::Flame => {
                        if self.at(x, y) == Mat::Empty && self.chance(3) {
                            self.set(x, y, m);
                        }
                    }
                    _ => {
                        if self.at(x, y) == Mat::Empty {
                            self.set(x, y, m);
                        }
                    }
                }
            }
        }
    }

    pub fn line(&mut self, a: (i32, i32), b: (i32, i32), r: i32, m: Mat) {
        let steps = (b.0 - a.0).abs().max((b.1 - a.1).abs()).max(1);
        for i in 0..=steps {
            let x = a.0 + (b.0 - a.0) * i / steps;
            let y = a.1 + (b.1 - a.1) * i / steps;
            self.paint(x, y, r, m);
        }
    }

    /// Blast: flames and smoke in a disc, loose stuff gets blown away.
    pub fn explode(&mut self, cx: i32, cy: i32, r: i32) {
        if self.blasts.len() < 8 {
            self.blasts.push((cx, cy, r));
        }
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy > r * r {
                    continue;
                }
                let (x, y) = (cx + dx, cy + dy);
                let m = self.raw(x, y);
                if matches!(m.kind(), Kind::Powder | Kind::Liquid | Kind::Gas | Kind::Flame) || m == Mat::Empty {
                    match self.rand() % 3 {
                        0 => self.set(x, y, Mat::Fire),
                        1 => self.set(x, y, Mat::Steam),
                        _ => self.set(x, y, Mat::Empty),
                    }
                } else if m == Mat::Wood && self.chance(3) {
                    self.set(x, y, Mat::Ember);
                } else if m == Mat::Ice {
                    self.set(x, y, Mat::Water);
                }
            }
        }
    }

    // ----- generator --------------------------------------------------------

    fn fill_rect(&mut self, x0: i32, y0: i32, w: i32, h: i32, m: Mat) {
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                self.set(x, y, m);
            }
        }
    }

    /// Random hills, lakes and trees. Returns (entry, exit) positions.
    pub fn generate(&mut self, seed: u32) -> ((i32, i32), (i32, i32)) {
        self.clear();
        self.rng = seed.wrapping_mul(2654435761) | 1;
        let p: Vec<f32> = (0..6).map(|_| (self.rand() % 6283) as f32 / 1000.0).collect();
        let mut hs = vec![0i32; W];
        for x in 0..W {
            let xf = x as f32 / K as f32;
            let mut h = 112.0 + 6.0 * (xf * 0.018 + p[0]).sin() + 3.0 * (xf * 0.047 + p[1]).sin()
                + 1.0 * (xf * 0.11 + p[2]).sin();
            let edge = (x.min(W - 1 - x) as f32 / (40 * K) as f32).min(1.0);
            let edge = edge * edge * (3.0 - 2.0 * edge);
            h = 108.0 + (h - 108.0) * edge;
            hs[x] = (h * K as f32) as i32;
        }
        let sea = 122 * K;
        for x in 0..W {
            for y in hs[x]..H as i32 {
                let m = if y < hs[x] + 3 * K { Mat::Sand } else { Mat::Stone };
                self.set(x as i32, y, m);
            }
            for y in sea..hs[x] {
                self.set(x as i32, y, Mat::Water);
            }
        }
        for _ in 0..5 {
            let x = (50 * K) + (self.rand() % (W as u32 - 100 * K as u32)) as i32;
            let base = hs[x as usize];
            if base >= sea {
                continue;
            }
            let hgt = (8 + (self.rand() % 7) as i32) * K;
            self.fill_rect(x, base - hgt, 2 * K, hgt, Mat::Wood);
            for k in 1..5 * K {
                self.fill_rect(x - k, base - hgt + k, 1, K, Mat::Wood);
                self.fill_rect(x + 2 * K + k - 1, base - hgt + k, 1, K, Mat::Wood);
            }
        }
        ((16 * K, hs[16 * K as usize] - 8 * K), (W as i32 - 16 * K, hs[W - 16 * K as usize] - 8 * K))
    }

    // ----- simulation -------------------------------------------------------

    pub fn step(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        let f = self.frame;
        let ltr = f & 1 == 0;
        for y in (0..H as i32).rev() {
            for i in 0..W as i32 {
                let x = if ltr { i } else { W as i32 - 1 - i };
                let c = self.cells[Self::idx(x, y)];
                if c.m == Mat::Empty || c.stamp == f {
                    continue;
                }
                match c.m {
                    Mat::Sand | Mat::Rust | Mat::Ash => self.step_powder(x, y),
                    Mat::Gunpowder => self.step_gunpowder(x, y),
                    Mat::Water => self.step_water(x, y),
                    Mat::Steam => self.step_steam(x, y),
                    Mat::Fire => self.step_fire(x, y),
                    Mat::Ember => self.step_ember(x, y),
                    Mat::Lava => self.step_lava(x, y),
                    Mat::Iron => self.step_iron(x, y),
                    Mat::Ice => self.step_ice(x, y),
                    _ => {}
                }
            }
        }
        if f & 1 == 0 {
            self.level_water();
        }
    }

    /// Cheap hydrostatic pressure: inside one connected body of water, a
    /// calm surface that sits well above another one gives a cell to it.
    /// That makes water climb back up in U-shaped tubes (communicating vessels).
    fn level_water(&mut self) {
        let mut seen = std::mem::take(&mut self.seen);
        let mut stack = std::mem::take(&mut self.stack);
        seen.fill(false);
        for start in 0..W * H {
            if seen[start] || self.cells[start].m != Mat::Water {
                continue;
            }
            let mut donor: Option<(i32, i32)> = None;
            let mut recv: Option<(i32, i32)> = None;
            seen[start] = true;
            stack.push(start);
            while let Some(i) = stack.pop() {
                let (x, y) = ((i % W) as i32, (i / W) as i32);
                if self.at(x, y - 1) == Mat::Empty && self.at(x, y + 1) == Mat::Water {
                    if recv.map_or(true, |r| y > r.1) {
                        recv = Some((x, y));
                    }
                    if self.at(x - 1, y) != Mat::Empty
                        && self.at(x + 1, y) != Mat::Empty
                        && donor.map_or(true, |d| y < d.1)
                    {
                        donor = Some((x, y));
                    }
                }
                for (nx, ny) in Self::neighbors(x, y) {
                    if Self::inb(nx, ny) {
                        let j = Self::idx(nx, ny);
                        if !seen[j] && self.cells[j].m == Mat::Water {
                            seen[j] = true;
                            stack.push(j);
                        }
                    }
                }
            }
            if let (Some(d), Some(r)) = (donor, recv) {
                if r.1 - d.1 >= 3 {
                    self.set(d.0, d.1, Mat::Empty);
                    self.set(r.0, r.1 - 1, Mat::Water);
                }
            }
        }
        self.seen = seen;
        self.stack = stack;
    }

    fn neighbors(x: i32, y: i32) -> [(i32, i32); 4] {
        [(x, y - 1), (x + 1, y), (x, y + 1), (x - 1, y)]
    }

    /// A flame touches a neighbour that can burn: a loose wood cell becomes
    /// an ember, a wooden block heats up.
    fn ignite(&mut self, nx: i32, ny: i32, one_in: u32) {
        if !Self::inb(nx, ny) || !self.chance(one_in) {
            return;
        }
        let b = self.mask[Self::idx(nx, ny)];
        if b != 0 {
            if self.block_mat[b as usize - 1] == Mat::Wood {
                self.touched.push(b - 1);
            }
        } else if self.cells[Self::idx(nx, ny)].m == Mat::Wood {
            self.set(nx, ny, Mat::Ember);
        }
    }

    fn step_powder(&mut self, x: i32, y: i32) {
        let d = self.dir();
        for dx in [0, d, -d] {
            if matches!(self.at(x + dx, y + 1).kind(), Kind::Empty | Kind::Liquid | Kind::Gas) {
                self.swap((x, y), (x + dx, y + 1));
                return;
            }
        }
    }

    fn step_gunpowder(&mut self, x: i32, y: i32) {
        for (nx, ny) in Self::neighbors(x, y) {
            if self.at(nx, ny).hot() {
                self.set(x, y, Mat::Fire);
                self.explode(x, y, 7 * K / 2 + 4);
                return;
            }
        }
        self.step_powder(x, y);
    }

    fn step_water(&mut self, x: i32, y: i32) {
        let d = self.dir();
        for dx in [0, d, -d] {
            if matches!(self.at(x + dx, y + 1).kind(), Kind::Empty | Kind::Gas) {
                self.swap((x, y), (x + dx, y + 1));
                return;
            }
        }
        for dir in [d, -d] {
            let mut k = 0;
            while k < 4 * K && self.at(x + dir * (k + 1), y) == Mat::Empty {
                k += 1;
            }
            if k > 0 {
                self.swap((x, y), (x + dir * k, y));
                return;
            }
        }
    }

    fn step_steam(&mut self, x: i32, y: i32) {
        let limit = if y < 3 * K { 120 } else { 900 };
        if self.chance(limit) {
            self.set(x, y, Mat::Water);
            return;
        }
        let d = self.dir();
        let order: [(i32, i32); 3] = if self.rand() % 10 < 7 {
            [(0, -1), (d, -1), (d, 0)]
        } else {
            [(d, 0), (-d, 0), (0, -1)]
        };
        for (dx, dy) in order {
            if self.at(x + dx, y + dy) == Mat::Empty {
                self.swap((x, y), (x + dx, y + dy));
                return;
            }
        }
    }

    fn step_fire(&mut self, x: i32, y: i32) {
        let i = Self::idx(x, y);
        if self.cells[i].life == 0 {
            self.set(x, y, Mat::Empty);
            return;
        }
        self.cells[i].life -= 1;
        for (nx, ny) in Self::neighbors(x, y) {
            match self.at(nx, ny) {
                Mat::Water => {
                    self.set(x, y, Mat::Steam);
                    if self.chance(2) {
                        self.set(nx, ny, Mat::Steam);
                    }
                    return;
                }
                Mat::Wood => self.ignite(nx, ny, 25),
                _ => {}
            }
        }
        let d = self.dir();
        let r = self.rand() % 10;
        let order: [(i32, i32); 2] = if r < 6 { [(0, -1), (d, -1)] } else if r < 8 { [(d, 0), (-d, 0)] } else { return };
        for (dx, dy) in order {
            if self.at(x + dx, y + dy) == Mat::Empty {
                self.swap((x, y), (x + dx, y + dy));
                return;
            }
        }
    }

    fn step_ember(&mut self, x: i32, y: i32) {
        let i = Self::idx(x, y);
        if self.cells[i].life == 0 {
            let m = if self.chance(2) { Mat::Ash } else { Mat::Empty };
            self.set(x, y, m);
            return;
        }
        self.cells[i].life -= 1;
        for (nx, ny) in Self::neighbors(x, y) {
            match self.at(nx, ny) {
                Mat::Water => {
                    self.set(x, y, Mat::Ash);
                    if self.chance(3) {
                        self.set(nx, ny, Mat::Steam);
                    }
                    return;
                }
                Mat::Wood => self.ignite(nx, ny, 80),
                _ => {}
            }
        }
        if self.at(x, y - 1) == Mat::Empty && self.chance(12) {
            self.set(x, y - 1, Mat::Fire);
        }
    }

    fn step_lava(&mut self, x: i32, y: i32) {
        for (nx, ny) in Self::neighbors(x, y) {
            match self.at(nx, ny) {
                Mat::Water => {
                    self.set(x, y, Mat::Stone);
                    if self.chance(4) {
                        self.set(nx, ny, Mat::Steam);
                    }
                    return;
                }
                Mat::Wood => self.ignite(nx, ny, 20),
                Mat::Ice => {
                    if self.chance(6) {
                        self.set(nx, ny, Mat::Water);
                    }
                }
                _ => {}
            }
        }
        if self.at(x, y - 1) == Mat::Empty && self.chance(150) {
            self.set(x, y - 1, Mat::Fire);
        }
        if self.chance(2) {
            return;
        }
        let d = self.dir();
        for dx in [0, d, -d] {
            if matches!(self.at(x + dx, y + 1).kind(), Kind::Empty | Kind::Gas) {
                self.swap((x, y), (x + dx, y + 1));
                return;
            }
        }
        if self.chance(3) {
            for dir in [d, -d] {
                if self.at(x + dir, y) == Mat::Empty {
                    self.swap((x, y), (x + dir, y));
                    return;
                }
            }
        }
    }

    fn step_ice(&mut self, x: i32, y: i32) {
        let n = Self::neighbors(x, y)[(self.rand() % 4) as usize];
        if self.at(n.0, n.1).hot() && self.chance(6) {
            self.set(x, y, Mat::Water);
        }
    }

    fn step_iron(&mut self, x: i32, y: i32) {
        let n = Self::neighbors(x, y)[(self.rand() % 4) as usize];
        if self.at(n.0, n.1) == Mat::Water && self.chance(300) {
            self.set(x, y, Mat::Rust);
        }
    }

    /// Premultiplied RGBA of a cell; empty cells are fully transparent so the
    /// background shows through.
    pub fn color(&self, x: usize, y: usize) -> [u8; 4] {
        let c = self.cells[y * W + x];
        if c.m == Mat::Empty {
            return [0, 0, 0, 0];
        }
        let h = (x as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663);
        let v = ((h >> 7) & 15) as i32 - 8;
        let f = self.frame as u32;
        let flick = ((h ^ f.wrapping_mul(2654435761)) >> 11 & 31) as i32;
        let (r, g, b, a): (i32, i32, i32, i32) = match c.m {
            Mat::Empty => (0, 0, 0, 0),
            Mat::Sand => (226 + v, 200 + v, 124 + v, 255),
            Mat::Water => (44 + v / 2, 124 + v + flick / 4, 224 + v, 205),
            Mat::Fire => (255, (80 + c.life as i32 * 3).min(235), 30, 235),
            Mat::Steam => (205 + v, 214 + v, 230 + v, 120 + flick),
            Mat::Iron => (146 + v, 156 + v, 174 + v, 255),
            Mat::Wood => (146 + v / 2 + (y as i32 % 3) * 6, 98 + v / 2, 54, 255),
            Mat::Stone => (108 + v, 108 + v, 120 + v, 255),
            Mat::Brick => (184 + v, 72 + v / 2, 56, 255),
            Mat::Rust => (156 + v, 84 + v / 2, 42, 255),
            Mat::Ash => (100 + v, 100 + v, 106 + v, 255),
            Mat::Ember => (255, 110 + flick * 3, 24, 255),
            Mat::Lava => (255, 78 + flick * 3 + v, 12, 255),
            Mat::Ice => (176 + v, 226 + v, 248, 225),
            Mat::Gunpowder => (62 + v, 58 + v, 66 + v + (flick / 8) * 6, 255),
        };
        let (r, g, b, a) = (r.clamp(0, 255), g.clamp(0, 255), b.clamp(0, 255), a.clamp(0, 255));
        [(r * a / 255) as u8, (g * a / 255) as u8, (b * a / 255) as u8, a as u8]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sand_falls_and_lands_on_stone() {
        let mut w = World::new();
        for x in 0..21 {
            w.set(x, 90, Mat::Stone);
        }
        w.set(10, 5, Mat::Sand);
        for _ in 0..300 {
            w.step();
        }
        assert_eq!(w.at(10, 89), Mat::Sand);
        assert_eq!(w.at(10, 5), Mat::Empty);
    }

    #[test]
    fn sand_vanishes_below_the_world() {
        let mut w = World::new();
        w.set(10, 5, Mat::Sand);
        for _ in 0..900 {
            w.step();
        }
        assert!(w.cells.iter().all(|c| c.m != Mat::Sand));
    }

    #[test]
    fn water_extinguishes_fire() {
        let mut w = World::new();
        for x in 0..20 {
            w.set(x, 100, Mat::Stone);
            w.set(x, 99, Mat::Water);
        }
        w.set(5, 98, Mat::Fire);
        for _ in 0..5 {
            w.step();
        }
        assert!((0..20).any(|x| (90..100).any(|y| w.at(x, y) == Mat::Steam)));
    }

    #[test]
    fn wood_burns_to_ash() {
        let mut w = World::new();
        for x in 0..10 {
            w.set(x, 100, Mat::Stone);
            w.set(x, 99, Mat::Wood);
        }
        w.set(0, 99, Mat::Ember);
        for _ in 0..3000 {
            w.step();
        }
        assert!((0..10).all(|x| w.at(x, 99) != Mat::Wood));
    }

    #[test]
    fn water_climbs_back_up_in_u_tube() {
        let mut w = World::new();
        for x in 9..=18 {
            w.set(x, 100, Mat::Stone);
        }
        for y in 40..=100 {
            w.set(9, y, Mat::Stone);
            w.set(18, y, Mat::Stone);
            if y < 98 {
                w.set(13, y, Mat::Stone);
                w.set(14, y, Mat::Stone);
            }
        }
        for y in 60..100 {
            for x in 10..=12 {
                w.set(x, y, Mat::Water);
            }
        }
        for _ in 0..1500 {
            w.step();
        }
        let right = (40..100).filter(|&y| (15..=17).any(|x| w.at(x, y) == Mat::Water)).count();
        assert!(right > 12, "right arm only {right} rows deep");
    }

    #[test]
    fn lava_meets_water_makes_stone() {
        let mut w = World::new();
        for x in 0..10 {
            w.set(x, 100, Mat::Stone);
        }
        for y in 90..100 {
            w.set(2, y, Mat::Stone);
            w.set(9, y, Mat::Stone);
        }
        for x in 3..9 {
            w.set(x, 99, Mat::Water);
            w.set(x, 98, Mat::Water);
        }
        w.set(5, 90, Mat::Lava);
        for _ in 0..60 {
            w.step();
        }
        assert!((3..9).any(|x| (96..100).any(|y| w.at(x, y) == Mat::Stone)));
    }

    #[test]
    fn water_drains_over_the_edge() {
        let mut w = World::new();
        for x in 0..10 {
            w.set(x, 100, Mat::Stone);
            w.set(x, 99, Mat::Water);
        }
        for _ in 0..300 {
            w.step();
        }
        assert!((0..10).all(|x| w.at(x, 99) != Mat::Water));
    }

    #[test]
    fn lava_melts_ice() {
        let mut w = World::new();
        for x in 0..10 {
            w.set(x, 100, Mat::Stone);
            w.set(x, 99, Mat::Ice);
        }
        w.set(5, 98, Mat::Lava);
        for _ in 0..200 {
            w.step();
        }
        assert!((0..10).any(|x| w.at(x, 99) != Mat::Ice));
    }

    #[test]
    fn gunpowder_explodes_on_fire() {
        let mut w = World::new();
        for x in 0..30 {
            w.set(x, 100, Mat::Stone);
            for y in 96..100 {
                w.set(x, y, Mat::Gunpowder);
            }
        }
        w.set(15, 95, Mat::Fire);
        for _ in 0..30 {
            w.step();
        }
        assert!(!w.blasts.is_empty() || w.cells.iter().all(|c| c.m != Mat::Gunpowder));
    }

    #[test]
    fn mask_blocks_sand_and_displaces_it() {
        let mut w = World::new();
        for x in 0..20 {
            w.set(x, 100, Mat::Stone);
        }
        w.set(10, 99, Mat::Sand);
        w.begin_blocks(vec![Mat::Brick]);
        for x in 8..13 {
            for y in 96..100 {
                w.mark_block(x, y, 0);
            }
        }
        w.end_blocks();
        assert_eq!(w.raw(10, 99), Mat::Empty);
        assert!((0..96).any(|y| w.raw(10, y) == Mat::Sand));
        assert_eq!(w.at(10, 98), Mat::Brick);
    }

    #[test]
    fn generator_makes_ground() {
        let mut w = World::new();
        let (e, _) = w.generate(7);
        assert_eq!(w.at(e.0, e.1), Mat::Empty);
        assert!(w.at(200, H as i32 - 2) == Mat::Stone);
    }
}
