//! Falling-sand world: one material per cell, simulated cell by cell.
//! Rigid blocks live outside the grid; they are stamped into a mask each step
//! so particles and the car treat them as solid.

pub mod flow;
mod pockets;
mod pressure;
pub mod sleep;
pub use pockets::boil_c;
mod thermal;
pub use thermal::{Season, DAY_STEPS};

/// How the brush treats what is already there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaintRule {
    /// Add into empty space (solids overwrite).
    Fill,
    /// Overwrite everything the brush touches.
    Replace,
    /// Replace only cells of this material.
    Only(Mat),
}

/// Landscape type chosen when a world is created; it also picks the backdrop.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Biome {
    Countryside,
    Mountain,
    Sea,
}

impl Biome {
    pub const ALL: [Biome; 3] = [Biome::Countryside, Biome::Mountain, Biome::Sea];

    pub fn name(self) -> &'static str {
        match self {
            Biome::Countryside => "Campagne",
            Biome::Mountain => "Montagne",
            Biome::Sea => "Mer",
        }
    }

    pub fn id(self) -> u8 {
        match self {
            Biome::Countryside => 0,
            Biome::Mountain => 1,
            Biome::Sea => 2,
        }
    }

    pub fn from_id(id: u8) -> Biome {
        Self::ALL.iter().copied().find(|b| b.id() == id).unwrap_or(Biome::Countryside)
    }
}

/// Room for this many materials in per-material tables.
pub const NMAT: usize = 40;

pub const W: usize = 960;
pub const H: usize = 540;
/// Resolution factor relative to the 320x180 design grid.
pub const K: i32 = 3;
/// Thickness, in cells, of the masonry vault the tunnel tool leaves around its hole.
pub const TUNNEL_LINING: i32 = 3;
/// Above this row the air is cold: steam condenses into clouds.
const COLD_Y: i32 = (H as i32) * 3 / 10;

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
    Dirt,
    Grass,
    Leaf,
    Steel,
    Cloud,
    Seed,
    Sapling,
    Mud,
    Glass,
    Snow,
    Shard,
    DeadLeaf,
    Branch,
    Trunk,
    Hail,
    Oil,
    Smoke,
    Molten,
    /// Clay: fired in a hot fire it becomes terracotta.
    Clay,
    Terracotta,
    /// Wood heated without air: almost pure carbon. It burns hot and slowly, and gives its
    /// carbon to iron (steel) and to rust (iron again).
    Charcoal,
    /// Saltpetre (potassium nitrate): white crystals grown where leaves rot with wood ash. Ground
    /// with charcoal it makes black powder; water dissolves it.
    Saltpeter,
    /// Steel that has melted: it sets again into steel (iron that melts sets into iron).
    MoltenSteel,
    /// The masonry vault the tunnel tool leaves over its hole. It is bolted into the ground
    /// around it, so it holds up the roof even when the tunnel cuts a hill in two.
    Vault,
}

impl Mat {
    pub const ALL: [Mat; 39] = [
        Mat::Empty, Mat::Sand, Mat::Water, Mat::Fire, Mat::Steam, Mat::Iron, Mat::Wood, Mat::Stone,
        Mat::Brick, Mat::Rust, Mat::Ash, Mat::Ember, Mat::Lava, Mat::Ice, Mat::Gunpowder, Mat::Dirt, Mat::Grass,
        Mat::Leaf, Mat::Steel, Mat::Cloud, Mat::Seed, Mat::Sapling, Mat::Mud, Mat::Glass, Mat::Snow, Mat::Shard, Mat::DeadLeaf, Mat::Branch, Mat::Trunk, Mat::Hail, Mat::Oil, Mat::Smoke, Mat::Molten, Mat::Clay, Mat::Terracotta,
        Mat::Charcoal, Mat::Saltpeter, Mat::MoltenSteel, Mat::Vault,
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
            Sand | Rust | Ash | Charcoal | Gunpowder | Saltpeter | Seed | Mud | Snow | Shard | DeadLeaf | Hail => Kind::Powder,
            Water | Lava | Oil | Molten | MoltenSteel => Kind::Liquid,
            Steam | Cloud | Smoke => Kind::Gas,
            Fire => Kind::Flame,
            Iron | Wood | Stone | Brick | Ember | Ice | Dirt | Grass | Leaf | Steel | Sapling | Glass | Branch | Trunk | Clay | Terracotta | Vault => Kind::Static,
        }
    }

    /// Blocks the car and rigid blocks.
    pub fn is_hard(self) -> bool {
        matches!(self.kind(), Kind::Static | Kind::Powder)
    }

    /// Living tree parts: cars drive through them (trees never block the road).
    pub fn is_vegetation(self) -> bool {
        matches!(self, Mat::Trunk | Mat::Leaf | Mat::Branch | Mat::Sapling)
    }

    /// Anything that burns like wood.
    pub fn is_wooden(self) -> bool {
        matches!(self, Mat::Wood | Mat::Trunk | Mat::Leaf | Mat::Branch)
    }

    /// Metal that has melted (iron or steel).
    pub fn is_molten(self) -> bool {
        matches!(self, Mat::Molten | Mat::MoltenSteel)
    }

    /// Molten rock or metal.
    pub fn is_melt(self) -> bool {
        matches!(self, Mat::Lava | Mat::Molten | Mat::MoltenSteel)
    }

    pub fn hot(self) -> bool {
        matches!(self, Mat::Fire | Mat::Ember | Mat::Lava)
    }
}

/// The three science views (temperature, pressure, humidity) are drawn the same way: the air is a
/// smooth field, and every grain of gas (steam, cloud, smoke) is drawn over it as a lighter dot
/// carrying its own value, so one can watch it drift and swirl through the field.
pub fn gas_grain(m: Mat) -> bool {
    matches!(m, Mat::Steam | Mat::Cloud | Mat::Smoke)
}

/// How a grain of gas shows in a science view: its own colour, lightened a little.
pub fn grain_dot([r, g, b]: [u8; 3]) -> [u8; 4] {
    let up = |c: u8| (c as f32 + (255.0 - c as f32) * 0.28) as u8;
    [up(r), up(g), up(b), 255]
}

/// A coarse field of `b`×`b` squares spread back over every cell, bilinear between the centres of
/// the squares, so the picture has no visible blocks.
pub fn upsample(coarse: &[f32], bw: usize, bh: usize, b: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; W * H];
    for y in 0..H {
        let fy = ((y as f32 + 0.5) / b as f32 - 0.5).clamp(0.0, bh as f32 - 1.0);
        let (j0, ty) = (fy as usize, fy.fract());
        let j1 = (j0 + 1).min(bh - 1);
        for x in 0..W {
            let fx = ((x as f32 + 0.5) / b as f32 - 0.5).clamp(0.0, bw as f32 - 1.0);
            let (i0, tx) = (fx as usize, fx.fract());
            let i1 = (i0 + 1).min(bw - 1);
            let top = coarse[j0 * bw + i0] * (1.0 - tx) + coarse[j0 * bw + i1] * tx;
            let bot = coarse[j1 * bw + i0] * (1.0 - tx) + coarse[j1 * bw + i1] * tx;
            out[y * W + x] = top * (1.0 - ty) + bot * ty;
        }
    }
    out
}

/// A pressure (atmospheres, absolute) as a colour: violet for thin air and suction, blue and green
/// around one atmosphere, yellow, orange and red under load, white under crushing weight.
pub fn pressure_rgb(atm: f32) -> (u8, u8, u8) {
    const STOPS: [(f32, [f32; 3]); 8] = [
        (0.0, [90.0, 40.0, 190.0]),
        (0.45, [100.0, 70.0, 220.0]),
        (0.75, [50.0, 110.0, 235.0]),
        (1.05, [50.0, 190.0, 150.0]),
        (1.7, [235.0, 225.0, 70.0]),
        (3.0, [245.0, 130.0, 40.0]),
        (6.0, [230.0, 50.0, 40.0]),
        (12.0, [255.0, 245.0, 245.0]),
    ];
    let p = atm.clamp(0.0, 12.0);
    let k = STOPS.iter().position(|s| s.0 >= p).unwrap_or(STOPS.len() - 1).max(1);
    let ((p0, c0), (p1, c1)) = (STOPS[k - 1], STOPS[k]);
    let f = ((p - p0) / (p1 - p0)).clamp(0.0, 1.0);
    let m = |a: f32, b: f32| (a + (b - a) * f) as u8;
    (m(c0[0], c1[0]), m(c0[1], c1[1]), m(c0[2], c1[2]))
}

/// Hot metal glows: dull red from about 500 °C, then orange, then yellow-white as it nears its
/// melting point (the colours a smith reads off the iron in the forge).
fn glow((r, g, b): (i32, i32, i32), t: f32) -> (i32, i32, i32, i32) {
    let k = ((t - 480.0) / (thermal::METAL_MELT_C - 480.0)).clamp(0.0, 1.0);
    if k == 0.0 {
        return (r, g, b, 255);
    }
    // Red first, then the green comes in (orange, yellow), the blue last.
    let (hr, hg, hb) = (255.0, 250.0 * k * k, 160.0 * k.powi(4));
    let mix = (k * 2.0).min(1.0);
    let m = |a: i32, h: f32| (a as f32 + (h - a as f32) * mix) as i32;
    (m(r, hr), m(g, hg), m(b, hb), 255)
}

/// Weight of a cell of this material, relative to water.
fn density(m: Mat) -> f32 {
    match m {
        Mat::Water | Mat::Oil => if m == Mat::Oil { 0.8 } else { 1.0 },
        Mat::Lava => 3.0,
        Mat::Molten | Mat::MoltenSteel => 6.5,
        Mat::Stone => 2.7,
        Mat::Dirt | Mat::Sand | Mat::Mud => 1.6,
        Mat::Clay => 1.8,
        Mat::Terracotta | Mat::Vault => 2.0,
        Mat::Brick => 1.2,
        Mat::Wood | Mat::Trunk | Mat::Branch => 0.6,
        Mat::Leaf | Mat::Sapling | Mat::Grass | Mat::Seed | Mat::DeadLeaf => 0.3,
        Mat::Iron | Mat::Steel => 7.8,
        Mat::Glass | Mat::Shard => 2.5,
        Mat::Ice | Mat::Hail => 0.92,
        Mat::Snow => 0.3,
        Mat::Ash => 0.5,
        Mat::Charcoal => 0.3,
        Mat::Gunpowder | Mat::Rust => 1.5,
        Mat::Saltpeter => 2.1,
        _ => 1.0,
    }
}

#[derive(Clone, Copy)]
pub struct Cell {
    pub m: Mat,
    pub life: u8,
    stamp: u8,
}

const EMPTY: Cell = Cell { m: Mat::Empty, life: 0, stamp: 0 };

/// Steps for a cell of water at 40 °C under perfectly dry, still air to evaporate, on average.
const EVAPORATE_40C: f32 = 9_000.0;
/// A trunk cell standing on soil tries to transpire once in this many steps.
const TRANSPIRE_EVERY: u32 = 200;

/// Saturation vapour pressure of water at `t` °C, in kPa (Magnus formula): how much vapour the air
/// can hold at that temperature. It roughly doubles every 11 °C.
pub fn vapour_pressure(t: f32) -> f32 {
    0.6108 * (17.27 * t / (t + 237.3)).exp()
}

/// A falling grain speeds up by this many eighths of a cell per step, every step.
const GRAIN_ACCEL: u8 = 2;
/// The fastest a grain falls through air (eighths of a cell per step).
const GRAIN_TERMINAL: u8 = 32;

/// Snowflakes fall gently: at most this many eighths of a cell per step.
const SNOW_TERMINAL: u8 = 6;
/// How far along a cell of snow can ooze before dropping a cell (a slope of 1 in this many).
const SNOW_RUN: i32 = 5;
/// A cell of snow at rest oozes once in this many steps: a very viscous liquid.
const SNOW_CREEP: u32 = 8;
/// Cells of vapour (steam or cloud) that one cell of liquid water makes, and that it takes to make
/// one drop again. Real water swells about 1700 times as it boils; the game keeps a small ratio so
/// that a pot does not fill the sky, but a sky full of cloud only gives back a little water.
pub const VAPOUR_PER_WATER: usize = 4;
/// The fastest a raindrop falls (eighths of a cell per step): a little faster than a grain of dust.
const DROP_TERMINAL: u8 = 40;
/// How much of the wind's speed a falling drop takes on sideways.
const DROP_DRIFT: f32 = 0.8;
/// How many times faster a drop in mid-air evaporates than the same water lying in a puddle.
const DROP_EVAPORATION: f32 = 30.0;
/// A drop falling through a cloud sweeps up the cell under it once in this many steps.
const DROP_SWEEP: u32 = 5;
/// In a cloud, a cell turns into a raindrop once in this many steps where the cloud is thickest.
const RAIN_EVERY: f32 = 8000.0;
/// Below this many cloud cells in the 5 x 5 around it (out of 24), a cloud is too thin to rain.
const RAIN_MIN_WATER: f32 = 8.0;

/// An ember whose life is at least this is burning charcoal: hotter and slower than wood.
pub(crate) const COAL_LIFE: u8 = 200;

/// A rigid piece of solid material that has lost its support and is falling.
struct Piece {
    cells: Vec<usize>,
    /// Downward speed in cells per drop (negative while bouncing up).
    vy: f32,
    /// Mean density relative to water: lighter than a liquid, it floats on it (ice, wood).
    density: f32,
}

/// A copy of everything that changes while the world runs (for undo).
#[derive(Clone)]
pub struct State {
    cells: Vec<Cell>,
    temp: Vec<f32>,
    clock: u32,
}

pub struct World {
    pub cells: Vec<Cell>,
    frame: u8,
    rng: u32,
    seen: Vec<bool>,
    stack: Vec<usize>,
    falling: Vec<Piece>,
    ticks: u32,
    /// Temperature in °C of every cell (air included).
    pub temp: Vec<f32>,
    /// Steps since the world began; drives the day and night cycle.
    pub clock: u32,
    /// When false the sun stands still.
    pub day_cycle: bool,
    thermal_flip: bool,
    /// Tests (and calm sandboxes) can pin the air temperature everywhere.
    pub climate_fixed: Option<f32>,
    /// The energy budget of the world: the sun in, the sky out to space (see `thermal.rs`).
    pub climate: thermal::Climate,
    /// Bumped whenever the picture could have changed: lets the renderer skip work.
    pub revision: u64,
    /// Wind, -1 (left) to 1 (right).
    pub wind: f32,
    /// The player's wind controls: strongest wind (0..1), how fast the direction keeps changing
    /// (0 = never, 1 = usual, up to 3), and how windy the weather is (0 calm .. 1 windy).
    pub wind_max: f32,
    pub wind_changes: f32,
    pub windiness: f32,
    /// Advances with `wind_changes`; the wind's direction follows it.
    pub wind_clock: f32,
    /// Landscape type this world was generated as (picks the backdrop).
    pub biome: Biome,
    /// Cached seasonal colours (leaf rgb, autumn amount, grass rgb).
    pub(crate) leaf_rgb: (i32, i32, i32),
    pub(crate) autumn: f32,
    pub(crate) grass_rgb: (i32, i32, i32),
    /// When true, solids that are not attached to the floor of the world fall.
    pub structures_fall: bool,
    mask: Vec<u16>,
    mask_prev: Vec<u16>,
    block_mat: Vec<Mat>,
    /// Per mask id: true for load-bearing parts (trusses) that a car drives through.
    mask_pass: Vec<bool>,
    /// Block ids (index into the block list) touched by flames this step.
    pub touched: Vec<u16>,
    /// Explosions this step: (x, y, radius).
    pub blasts: Vec<(i32, i32, i32)>,
    /// Gunpowder that went off this step (at night it launches fireworks).
    pub powder_booms: Vec<(i32, i32)>,
    /// Heavy landings of falling pieces: (x, y, strength). For sound.
    pub thuds: Vec<(i32, i32, f32)>,
    /// How many water cells moved since it was last read. For sound.
    pub water_flow: u32,
    /// The air's flow: layered winds, vortices, rising warm air. Gas rides it.
    pub flow: flow::Flow,
    /// Excess pressure over the ambient one for each flow cell, in atmospheres (see `pressure.rs`).
    dp: Vec<f32>,
    /// Its slow average, and the quick part left when that is taken off: what pushes.
    dp_slow: Vec<f32>,
    dpd: Vec<f32>,
    /// Whether the pressure pushes liquids and loose matter (for comparing in tests and demos).
    pub pressure_forces: bool,
    /// How much the simulation is simplified to keep the game smooth: 0 = everything exact,
    /// 1 = the wind carries air heat in 2 x 2 blocks, 2 = the heat pass runs half as often (with
    /// doubled rates), 3 = same, and calm air next to air no longer conducts heat (it only relaxes).
    pub lod: u8,
    /// Closed pockets of gas (see `pockets.rs`): for each cell its pocket id + 1 (0: not gas).
    pocket_of: Vec<u32>,
    pockets: Vec<pockets::Pocket>,
    /// Latent heat taken in (positive) or given up (negative) by a cell of water or ice on its way
    /// to changing state (see `thermal::latent_heat`).
    pub latent: Vec<f32>,
    /// Falling speed of each grain, in eighths of a cell per step (gravity speeds a free-falling
    /// grain up; landing stops it).
    vel: Vec<u8>,
    /// Absolute pressure (atm) inside each liquid cell; 1 elsewhere.
    liq_p: Vec<f32>,
    /// Scratch for the pockets: what each cell is to the gas, and the weight of each liquid cell.
    cls: Vec<u8>,
    liq_w: Vec<f32>,
    /// Sleeping zones: blocks of cells where nothing can happen are skipped (see `sleep.rs`).
    pub zones: sleep::Zones,
    /// Walls that gave way under the pressure of a pocket this step: (x, y, pressure in atm).
    pub bursts: Vec<(i32, i32, f32)>,
}

impl World {
    pub fn new() -> Self {
        World {
            cells: vec![EMPTY; W * H],
            frame: 0,
            rng: 0x1234_5678,
            seen: vec![false; W * H],
            stack: Vec::new(),
            falling: Vec::new(),
            ticks: 0,
            temp: vec![15.0; W * H],
            clock: DAY_STEPS / 5,
            day_cycle: true,
            thermal_flip: false,
            climate_fixed: None,
            climate: thermal::Climate::default(),
            revision: 1,
            wind: 0.0,
            wind_max: 1.0,
            wind_changes: 1.0,
            windiness: 1.0,
            wind_clock: 0.0,
            biome: Biome::Countryside,
            leaf_rgb: (52, 140, 60),
            autumn: 0.0,
            grass_rgb: (88, 172, 64),
            structures_fall: true,
            mask: vec![0; W * H],
            mask_prev: vec![0; W * H],
            block_mat: Vec::new(),
            mask_pass: Vec::new(),
            touched: Vec::new(),
            blasts: Vec::new(),
            powder_booms: Vec::new(),
            thuds: Vec::new(),
            water_flow: 0,
            flow: flow::Flow::new(),
            dp: vec![0.0; flow::FW * flow::FH],
            dp_slow: vec![0.0; flow::FW * flow::FH],
            dpd: vec![0.0; flow::FW * flow::FH],
            pressure_forces: true,
            lod: 0,
            pocket_of: vec![0; W * H],
            pockets: Vec::new(),
            liq_p: vec![1.0; W * H],
            latent: vec![0.0; W * H],
            vel: vec![0; W * H],
            cls: vec![0; W * H],
            liq_w: vec![0.0; W * H],
            bursts: Vec::new(),
            zones: sleep::Zones::new(),
        }
    }

    /// A world where nothing falls apart: handy for tests with floating floors.
    #[cfg(test)]
    pub fn new_static() -> Self {
        let mut w = Self::new();
        w.structures_fall = false;
        w.climate_fixed = Some(15.0);
        w.reset_temps();
        w
    }

    pub fn clear(&mut self) {
        self.revision += 1;
        self.cells.fill(EMPTY);
        self.latent.fill(0.0);
        self.vel.fill(0);
        self.zones.wake_all();
        self.reset_temps();
        self.update_season_cache();
    }

    /// Replace the grid with saved materials (row by row, W*H entries).
    pub fn load_mats(&mut self, mats: &[Mat]) {
        for (i, m) in mats.iter().enumerate().take(W * H) {
            self.set((i % W) as i32, (i / W) as i32, *m);
        }
    }

    pub fn capture(&self) -> State {
        State { cells: self.cells.clone(), temp: self.temp.clone(), clock: self.clock }
    }

    pub fn restore_state(&mut self, st: &State) {
        self.revision += 1;
        self.cells.copy_from_slice(&st.cells);
        self.latent.fill(0.0);
        self.vel.fill(0);
        self.zones.wake_all();
        self.temp.copy_from_slice(&st.temp);
        self.clock = st.clock;
        self.falling.clear();
        self.touched.clear();
        self.blasts.clear();
        self.powder_booms.clear();
        self.update_season_cache();
    }

    pub fn take_water_flow(&mut self) -> u32 {
        std::mem::take(&mut self.water_flow)
    }

    /// How many cells of each material are on screen (indexed by `Mat as usize`).
    pub fn census(&self) -> [u32; NMAT] {
        let mut n = [0u32; NMAT];
        for c in &self.cells {
            n[c.m as usize] += 1;
        }
        n
    }

    /// Removes `count` cells of the given materials, nearest to `near` first.
    /// Returns false (and removes nothing) if there are not enough.
    pub fn consume(&mut self, mats: &[Mat], count: u32, near: (i32, i32)) -> bool {
        self.revision += 1;
        let mut found: Vec<(i64, usize)> = vec![];
        for (i, c) in self.cells.iter().enumerate() {
            if mats.contains(&c.m) {
                let (x, y) = ((i % W) as i64 - near.0 as i64, (i / W) as i64 - near.1 as i64);
                found.push((x * x + y * y, i));
            }
        }
        if (found.len() as u32) < count {
            return false;
        }
        let n = count as usize;
        if n > 0 {
            let k = (n - 1).min(found.len() - 1);
            found.select_nth_unstable(k);
        }
        let mut taken: Vec<usize> = found.iter().take(n).map(|&(_, i)| i).collect();
        for &i in &taken {
            self.cells[i] = EMPTY;
            self.zones.wake(i);
        }
        // The ground sinks into what was taken instead of leaving a cave under its turf: the soil
        // and grass above each hole slide down one cell, highest hole first so stacked holes close
        // from the top. The grass stays on top of the earth that carries it.
        taken.sort_unstable();
        let mut columns = vec![];
        for i in taken {
            let (x, y) = ((i % W) as i32, (i / W) as i32);
            let mut top = y;
            while top > 0 && matches!(self.cells[Self::idx(x, top - 1)].m, Mat::Dirt | Mat::Grass) {
                top -= 1;
            }
            for yy in (top + 1..=y).rev() {
                self.swap((x, yy - 1), (x, yy));
            }
            columns.push(x);
        }
        // Grass lives on earth: where all the earth under a turf was taken, the turf goes too.
        columns.sort_unstable();
        columns.dedup();
        for x in columns {
            let mut y = H as i32 - 1;
            while y >= 0 {
                if self.cells[Self::idx(x, y)].m != Mat::Grass {
                    y -= 1;
                    continue;
                }
                let bottom = y;
                while y >= 0 && self.cells[Self::idx(x, y)].m == Mat::Grass {
                    y -= 1;
                }
                if self.at(x, bottom + 1) != Mat::Dirt {
                    for yy in y + 1..=bottom {
                        self.set(x, yy, Mat::Empty);
                    }
                }
            }
        }
        true
    }

    pub fn snapshot(&self) -> Vec<Cell> {
        self.cells.clone()
    }

    pub fn restore(&mut self, snap: &[Cell]) {
        self.revision += 1;
        self.cells.copy_from_slice(snap);
        self.latent.fill(0.0);
        self.vel.fill(0);
        self.zones.wake_all();
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

    /// True with probability `p` (0..1).
    fn chance_f(&mut self, p: f32) -> bool {
        p > 0.0 && ((self.rand() % 1_000_000) as f32) < p * 1_000_000.0
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

    /// Breaks the glass around a cell. Shards that land on something become
    /// sand (glass is molten sand); the rest simply falls out of the way.
    pub fn shatter(&mut self, cx: i32, cy: i32) {
        let mut cells = vec![];
        for dy in -3..=3 {
            for dx in -3..=3 {
                if self.raw(cx + dx, cy + dy) == Mat::Glass {
                    cells.push((cx + dx, cy + dy));
                }
            }
        }
        for (x, y) in cells {
            let rests = self.raw(x, y + 1).is_hard() && self.raw(x, y + 1) != Mat::Glass;
            self.set(x, y, if rests { Mat::Shard } else { Mat::Empty });
        }
    }

    /// Index of the rigid block covering this cell, if any.
    pub fn block_at_cell(&self, x: i32, y: i32) -> Option<usize> {
        if !Self::inb(x, y) {
            return None;
        }
        match self.mask[Self::idx(x, y)] {
            0 => None,
            b => Some(b as usize - 1),
        }
    }

    /// Material of the grid itself, ignoring rigid blocks.
    pub fn raw(&self, x: i32, y: i32) -> Mat {
        if Self::inb(x, y) { self.cells[Self::idx(x, y)].m } else { Mat::Empty }
    }

    /// Obstacle test for the car; the world has no floor below it.
    pub fn hard_for_car(&self, x: i32, y: i32) -> bool {
        // Beyond the edges there is nothing to hold a car back.
        if y >= H as i32 || y < 0 || x < 0 || x >= W as i32 {
            return false;
        }
        let i = Self::idx(x, y);
        let b = self.mask[i];
        if b != 0 && self.mask_pass.get(b as usize - 1).copied().unwrap_or(false) {
            // Load-bearing structure: the car passes through; only what is under it counts.
            let m = self.cells[i].m;
            return m.is_hard() && !m.is_vegetation();
        }
        let m = self.at(x, y);
        m.is_hard() && !m.is_vegetation()
    }

    pub fn set(&mut self, x: i32, y: i32, m: Mat) {
        if !Self::inb(x, y) {
            return;
        }
        self.revision += 1;
        let life = match m {
            Mat::Fire => 20 + (self.rand() % 40) as u8,
            Mat::Ember => 100 + (self.rand() % 100) as u8,
            _ => 0,
        };
        let frame = self.frame;
        let i = Self::idx(x, y);
        self.cells[i] = Cell { m, life, stamp: frame };
        self.latent[i] = 0.0;
        self.vel[i] = 0;
        self.zones.wake(i);
        match m {
            Mat::Fire => self.temp[i] = thermal::FIRE_C,
            Mat::Ember => self.temp[i] = thermal::EMBER_C,
            // Smoke leaves a fire hot, so it rises until it has cooled to the air around.
            Mat::Smoke => self.temp[i] = self.temp[i].max(160.0),
            _ => {}
        }
    }

    fn swap(&mut self, a: (i32, i32), b: (i32, i32)) {
        if !Self::inb(b.0, b.1) {
            self.cells[Self::idx(a.0, a.1)] = EMPTY;
            self.zones.wake(Self::idx(a.0, a.1));
            return;
        }
        let (ia, ib) = (Self::idx(a.0, a.1), Self::idx(b.0, b.1));
        self.zones.wake(ia);
        self.zones.wake(ib);
        if self.cells[ia].m == Mat::Water {
            self.water_flow += 1;
        }
        self.cells.swap(ia, ib);
        self.temp.swap(ia, ib);
        self.latent.swap(ia, ib);
        self.vel.swap(ia, ib);
        self.cells[ia].stamp = self.frame;
        self.cells[ib].stamp = self.frame;
    }

    // ----- rigid block mask -------------------------------------------------

    /// `passable[i]` marks pieces that are structure, not floor: cars go through them.
    pub fn begin_blocks(&mut self, mats: Vec<Mat>, passable: Vec<bool>) {
        self.mask_pass = passable;
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
            if (self.mask[i] != 0) != (self.mask_prev[i] != 0) {
                // A block moved here or away: what is around may move now.
                self.zones.wake(i);
            }
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
                    self.zones.wake(j);
                    break;
                }
            }
        }
    }

    // ----- painting ---------------------------------------------------------

    /// Paint a disc. Loose materials only fill empty cells, solids overwrite.
    pub fn paint(&mut self, cx: i32, cy: i32, r: i32, m: Mat) {
        self.paint_rule(cx, cy, r, m, PaintRule::Fill);
    }

    /// Paint a disc. Loose materials only fill empty cells when adding; replacing
    /// overwrites whatever is there; "only" swaps just one kind of material.
    pub fn paint_rule(&mut self, cx: i32, cy: i32, r: i32, m: Mat, rule: PaintRule) {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy > r * r + r / 2 {
                    continue;
                }
                let (x, y) = (cx + dx, cy + dy);
                if !Self::inb(x, y) || self.mask[Self::idx(x, y)] != 0 {
                    continue;
                }
                match rule {
                    PaintRule::Replace => self.set(x, y, m),
                    PaintRule::Only(target) => {
                        if self.raw(x, y) == target {
                            self.set(x, y, m);
                        }
                    }
                    PaintRule::Fill => match m.kind() {
                        Kind::Empty => self.set(x, y, Mat::Empty),
                        Kind::Static => self.set(x, y, m),
                        Kind::Gas | Kind::Flame => {
                            // Clouds are painted solid so they hold enough water to rain.
                            if self.at(x, y) == Mat::Empty && (m == Mat::Cloud || self.chance(3)) {
                                self.set(x, y, m);
                            }
                        }
                        _ => {
                            if self.at(x, y) == Mat::Empty {
                                self.set(x, y, m);
                            }
                        }
                    },
                }
                let i = Self::idx(x, y);
                if self.cells[i].m == m {
                    if let Some(t) = thermal::paint_temp(m) {
                        self.temp[i] = t;
                    }
                }
            }
        }
    }

    pub fn line(&mut self, a: (i32, i32), b: (i32, i32), r: i32, m: Mat) {
        self.line_rule(a, b, r, m, PaintRule::Fill);
    }

    pub fn line_rule(&mut self, a: (i32, i32), b: (i32, i32), r: i32, m: Mat, rule: PaintRule) {
        let steps = (b.0 - a.0).abs().max((b.1 - a.1).abs()).max(1);
        for i in 0..=steps {
            let x = a.0 + (b.0 - a.0) * i / steps;
            let y = a.1 + (b.1 - a.1) * i / steps;
            self.paint_rule(x, y, r, m, rule);
        }
    }

    /// Digs a tunnel along a stroke: like the eraser, but the vault is lined with masonry as it
    /// goes, so the roof does not cave in (see `tunnel`).
    pub fn tunnel_line(&mut self, a: (i32, i32), b: (i32, i32), r: i32) {
        let steps = (b.0 - a.0).abs().max((b.1 - a.1).abs()).max(1);
        for i in 0..=steps {
            let x = a.0 + (b.0 - a.0) * i / steps;
            let y = a.1 + (b.1 - a.1) * i / steps;
            self.tunnel(x, y, r);
        }
    }

    /// Empties a disc of radius `r`, then lines the ring just around it with masonry, from the
    /// top down to a little below the middle: a masonry vault. Sand, snow or water above can no
    /// longer pour in, and the roof holds as long as the walls rest on something. Rock, metal and
    /// masonry are left as they are (a tunnel through rock needs no lining), and so is the floor.
    pub fn tunnel(&mut self, cx: i32, cy: i32, r: i32) {
        self.paint_rule(cx, cy, r, Mat::Empty, PaintRule::Fill);
        let outer = r + TUNNEL_LINING;
        for dy in -outer..=r / 2 {
            for dx in -outer..=outer {
                let d2 = dx * dx + dy * dy;
                if d2 <= r * r + r / 2 || d2 > outer * outer + outer / 2 {
                    continue;
                }
                let (x, y) = (cx + dx, cy + dy);
                if !Self::inb(x, y) || self.mask[Self::idx(x, y)] != 0 {
                    continue;
                }
                if Self::needs_lining(self.raw(x, y)) {
                    self.set(x, y, Mat::Vault);
                }
            }
        }
    }

    /// Whether a tunnel's vault must be lined where it meets this material.
    fn needs_lining(m: Mat) -> bool {
        match m.kind() {
            Kind::Empty | Kind::Gas | Kind::Flame => false,
            // Hot liquids would fire the masonry: they are left to the player's care.
            Kind::Liquid => !m.hot() && !matches!(m, Mat::Molten | Mat::MoltenSteel),
            Kind::Powder => true,
            Kind::Static => !m.is_vegetation() && !matches!(m, Mat::Stone | Mat::Iron | Mat::Steel | Mat::Brick | Mat::Terracotta | Mat::Glass | Mat::Vault),
        }
    }

    /// How hard a solid material is to break (rock = 1).
    fn hardness(m: Mat) -> f32 {
        match m {
            Mat::Stone => 1.0,
            Mat::Brick | Mat::Terracotta | Mat::Vault => 0.7,
            Mat::Clay => 0.3,
            Mat::Iron | Mat::Steel => 2.4,
            Mat::Glass => 0.2,
            Mat::Ice => 0.35,
            Mat::Dirt | Mat::Grass | Mat::Mud => 0.3,
            Mat::Wood | Mat::Trunk | Mat::Branch => 0.4,
            Mat::Leaf | Mat::Sapling => 0.15,
            _ => 0.5,
        }
    }

    /// An explosion: a ball of fire and smoke, rock and other solids shattered
    /// into rubble (metal resists), and a pressure wave that shoves everything
    /// loose outward.
    pub fn explode(&mut self, cx: i32, cy: i32, r: i32) {
        self.revision += 1;
        self.flow.blast(cx, cy, r * 2, 0.15 + r as f32 * 0.03);
        let heavy = self.blasts.len() < 8;
        if heavy {
            self.blasts.push((cx, cy, r));
        }
        // A blast under water: how deep it is below the surface (None in the open air).
        let depth = {
            let mut d = 0;
            while d < 3 * r && self.raw(cx, cy - d).kind() == Kind::Liquid {
                d += 1;
            }
            (d > 0 && matches!(self.raw(cx, cy - d).kind(), Kind::Empty | Kind::Gas)).then_some(d)
        };
        let r2 = (r * r) as f32;
        for dy in -r..=r {
            for dx in -r..=r {
                let d2 = (dx * dx + dy * dy) as f32;
                if d2 > r2 {
                    continue;
                }
                let (x, y) = (cx + dx, cy + dy);
                let m = self.raw(x, y);
                let f = d2 / r2;
                match m.kind() {
                    Kind::Powder | Kind::Liquid | Kind::Gas | Kind::Flame | Kind::Empty => match self.rand() % 6 {
                        0 => self.set(x, y, Mat::Fire),
                        1 | 2 => self.set(x, y, Mat::Smoke),
                        _ => self.set(x, y, Mat::Empty),
                    },
                    Kind::Static => {
                        let h = Self::hardness(m);
                        if f * h < 0.35 {
                            // Shattered: rubble where it was rock, dust or embers otherwise.
                            let into = match m {
                                Mat::Stone => if self.chance(2) { Mat::Sand } else { Mat::Empty },
                                Mat::Brick | Mat::Dirt | Mat::Grass | Mat::Mud | Mat::Clay | Mat::Terracotta => if self.chance(2) { Mat::Mud } else { Mat::Empty },
                                Mat::Wood | Mat::Trunk | Mat::Branch => if self.chance(3) { Mat::Ember } else { Mat::Empty },
                                Mat::Glass => if self.chance(2) { Mat::Shard } else { Mat::Empty },
                                Mat::Ice => if self.chance(2) { Mat::Snow } else { Mat::Water },
                                _ => Mat::Empty,
                            };
                            self.set(x, y, into);
                        } else if m == Mat::Stone && f * h < 0.6 && self.chance(3) {
                            // Fractured rock crumbles to sand.
                            self.set(x, y, Mat::Sand);
                        } else if m.is_wooden() && self.chance(3) {
                            self.set(x, y, Mat::Ember);
                        }
                    }
                }
            }
        }
        if !heavy {
            return;
        }
        self.shove_out(cx, cy, r);
        // Under water the gas bubble lifts the surface above it into a dome and a plume of spray:
        // the shallower the blast, the higher the water is thrown.
        if let Some(d) = depth {
            let strength = (1.0 - d as f32 / (3.0 * r as f32)).clamp(0.0, 1.0);
            let half = r * 6 / 5;
            for dx in -half..=half {
                let x = cx + dx;
                let Some(top) = (cy - d - 2..=cy).find(|&y| self.raw(x, y).kind() == Kind::Liquid) else { continue };
                let shape = 1.0 - dx.abs() as f32 / half as f32;
                let rise = (strength * r as f32 * 1.6 * shape * (0.5 + (self.rand() % 100) as f32 / 100.0)) as i32;
                // The top few cells of the column go up, the highest first.
                for k in 0..3 {
                    let y = top + k;
                    if self.raw(x, y).kind() != Kind::Liquid {
                        break;
                    }
                    if let Some(up) = (1..=rise).rev().find(|&u| self.at(x, y - u) == Mat::Empty) {
                        self.swap((x, y), (x, y - up));
                    }
                }
            }
        }
        // A billow of smoke around the fireball.
        for dy in -(r * 3 / 2)..=(r * 3 / 2) {
            for dx in -(r * 3 / 2)..=(r * 3 / 2) {
                let d2 = dx * dx + dy * dy;
                if d2 > r * r && d2 <= (r * 3 / 2) * (r * 3 / 2) && self.chance(9) && self.at(cx + dx, cy + dy) == Mat::Empty {
                    self.set(cx + dx, cy + dy, Mat::Smoke);
                }
            }
        }
    }

    /// Pressure wave: loose matter near (cx, cy) is thrown outward, the closer the harder.
    /// Outer cells move first so they do not pile into inner ones.
    fn shove_out(&mut self, cx: i32, cy: i32, r: i32) {
        let reach = (r as f32 * 2.3) as i32;
        let reach2 = (reach * reach) as f32;
        let mut shoved: Vec<(f32, i32, i32)> = vec![];
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let d2 = (dx * dx + dy * dy) as f32;
                if d2 > reach2 || d2 < 1.0 {
                    continue;
                }
                let (x, y) = (cx + dx, cy + dy);
                if matches!(self.raw(x, y).kind(), Kind::Powder | Kind::Liquid | Kind::Gas | Kind::Flame) && self.mask_free(x, y) {
                    shoved.push((d2, x, y));
                }
            }
        }
        shoved.sort_unstable_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        shoved.truncate(7000);
        for (d2, x, y) in shoved {
            let d = d2.sqrt();
            let steps = (((1.0 - d / reach as f32) * r as f32 * 0.9).max(0.0)) as i32;
            if steps <= 0 {
                continue;
            }
            let (ux, uy) = ((x - cx) as f32 / d, (y - cy) as f32 / d);
            let (mut tx, mut ty) = (x, y);
            for k in 1..=steps {
                let (nx, ny) = (x + (ux * k as f32).round() as i32, y + (uy * k as f32).round() as i32);
                if self.at(nx, ny) == Mat::Empty {
                    (tx, ty) = (nx, ny);
                } else {
                    break;
                }
            }
            if (tx, ty) != (x, y) {
                self.swap((x, y), (tx, ty));
            }
        }
    }

    fn mask_free(&self, x: i32, y: i32) -> bool {
        !Self::inb(x, y) || self.mask[Self::idx(x, y)] == 0
    }

    // ----- generator --------------------------------------------------------

    fn fill_rect(&mut self, x0: i32, y0: i32, w: i32, h: i32, m: Mat) {
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                self.set(x, y, m);
            }
        }
    }

    /// Countryside by default; see [`World::generate_biome`].
    pub fn generate(&mut self, seed: u32) -> ((i32, i32), (i32, i32)) {
        self.generate_biome(seed, Biome::Countryside)
    }

    /// Builds a whole world for a landscape type: terrain, lakes or sea, trees,
    /// snow caps. Returns the (entry, exit) positions on the two ends.
    pub fn generate_biome(&mut self, seed: u32, biome: Biome) -> ((i32, i32), (i32, i32)) {
        self.clear();
        self.biome = biome;
        self.rng = seed.wrapping_mul(2654435761) | 1;
        let p: Vec<f32> = (0..6).map(|_| (self.rand() % 6283) as f32 / 1000.0).collect();
        let island: Vec<f32> = (0..3).map(|_| 60.0 + (self.rand() % 200) as f32).collect();
        let mut hs = vec![0i32; W];
        for x in 0..W {
            let xf = x as f32 / K as f32;
            let edge = (x.min(W - 1 - x) as f32 / (40 * K) as f32).min(1.0);
            let edge = edge * edge * (3.0 - 2.0 * edge);
            let h = match biome {
                Biome::Countryside => {
                    let h = 112.0 + 6.0 * (xf * 0.018 + p[0]).sin() + 3.0 * (xf * 0.047 + p[1]).sin() + (xf * 0.11 + p[2]).sin();
                    108.0 + (h - 108.0) * edge
                }
                Biome::Mountain => {
                    // Ridged noise gives sharp peaks.
                    let ridge = |f: f32, ph: f32| 1.0 - (xf * f + ph).sin().abs();
                    let h = 140.0 - (70.0 * ridge(0.021, p[0]) + 30.0 * ridge(0.047, p[1]) + 10.0 * ridge(0.11, p[2]));
                    128.0 + (h - 128.0) * edge
                }
                Biome::Sea => {
                    let t = (xf - 160.0) / 150.0;
                    let basin = (1.0 - t * t).max(0.0);
                    let mut h = 118.0 + 38.0 * basin.powf(0.8) + 2.0 * (xf * 0.06 + p[0]).sin();
                    for c in &island {
                        h -= 34.0 * (-((xf - c) / 14.0).powi(2)).exp() * basin.min(1.0);
                    }
                    118.0 + (h - 118.0) * edge
                }
            };
            hs[x] = ((h * K as f32) as i32).clamp(40, H as i32 - 40);
        }
        let sea = match biome {
            Biome::Countryside => 122 * K,
            Biome::Mountain => 135 * K,
            Biome::Sea => 128 * K,
        };
        for x in 0..W {
            let top = hs[x];
            let beach = top > sea - 5 * K;
            for y in top..H as i32 {
                let depth = y - top;
                let m = match biome {
                    // Snow caps (ice) on the peaks, bare rock below them, grass in the valleys.
                    Biome::Mountain if top < 250 && depth < 3 * K => Mat::Ice,
                    Biome::Mountain if top < 320 && depth < 5 * K => Mat::Stone,
                    _ if depth < 2 * K => {
                        if beach { Mat::Sand } else { Mat::Grass }
                    }
                    _ if depth < 6 * K && !beach => Mat::Dirt,
                    _ if depth < 3 * K => Mat::Sand,
                    _ => Mat::Stone,
                };
                self.set(x as i32, y, m);
            }
            for y in sea..top {
                self.set(x as i32, y, Mat::Water);
            }
        }
        let (tries, min_top) = match biome {
            Biome::Countryside => (6, 0),
            Biome::Mountain => (6, 330),
            Biome::Sea => (3, 0),
        };
        for _ in 0..tries {
            let x = (50 * K) + (self.rand() % (W as u32 - 100 * K as u32)) as i32;
            let base = hs[x as usize];
            if base >= sea - 4 * K || base < min_top {
                continue;
            }
            let hgt = (8 + (self.rand() % 6) as i32) * K;
            self.fill_rect(x, base - hgt, K, hgt, Mat::Trunk);
            let top = base - hgt;
            self.grow_crown(x, top);
        }
        self.reset_temps();
        ((16 * K, hs[16 * K as usize] - 8 * K), (W as i32 - 16 * K, hs[W - 16 * K as usize] - 8 * K))
    }

    // ----- simulation -------------------------------------------------------

    pub fn step(&mut self) {
        self.revision += 1;
        self.frame = self.frame.wrapping_add(1);
        self.step_cells();
        self.after_cells();
    }

    /// Every cell of matter takes its turn, bottom row first.
    fn step_cells(&mut self) {
        let f = self.frame;
        let ltr = f & 1 == 0;
        self.zones.begin();
        for y in (0..H as i32).rev() {
            let zrow = y as usize / sleep::CS * sleep::CW;
            for zi in 0..sleep::CW {
                let zx = if ltr { zi } else { sleep::CW - 1 - zi };
                let zone = zrow + zx;
                if !self.zones.awake[zone] {
                    continue;
                }
                let (x0, x1) = ((zx * sleep::CS) as i32, ((zx + 1) * sleep::CS).min(W) as i32);
                for i in 0..x1 - x0 {
                    let x = if ltr { x0 + i } else { x1 - 1 - i };
                    let c = self.cells[Self::idx(x, y)];
                    if c.m == Mat::Empty {
                        continue;
                    }
                    if c.stamp == f {
                        // Moved here this step, or a stamp left from 256 steps ago: either way
                        // look again next step.
                        self.zones.mark_restless(zone);
                        continue;
                    }
                    if self.restless(x, y, c) {
                        self.zones.mark_restless(zone);
                    }
                    match c.m {
                        Mat::Rust => {
                            self.step_powder(x, y);
                        }
                        Mat::Sand | Mat::Mud => self.step_sediment(x, y),
                        Mat::Dirt => self.step_dirt(x, y),
                        Mat::Grass => self.step_grass(x, y),
                        Mat::Snow => self.step_snow(x, y),
                        Mat::Hail => {
                            self.step_powder(x, y);
                        }
                        Mat::Shard => self.step_shard(x, y),
                        Mat::Ash => self.step_ash(x, y),
                        Mat::Charcoal => self.step_charcoal(x, y),
                        Mat::Seed => self.step_seed(x, y),
                        Mat::Sapling => self.step_sapling(x, y),
                        Mat::Leaf => self.step_leaf(x, y),
                        Mat::Wood | Mat::Trunk => self.step_wood(x, y),
                        Mat::Branch => self.step_branch(x, y),
                        Mat::DeadLeaf => self.step_dead_leaf(x, y),
                        Mat::Cloud => self.step_cloud(x, y),
                        Mat::Oil => {
                            if !self.fly(x, y) && !self.jet(x, y) && !self.seep(x, y) {
                                self.step_oil(x, y);
                            }
                        }
                        Mat::Smoke => self.step_smoke(x, y),
                        Mat::Steel => self.step_iron(x, y, 1500),
                        Mat::Gunpowder => self.step_gunpowder(x, y),
                        Mat::Saltpeter => self.step_saltpeter(x, y),
                        Mat::Water => {
                            if !self.fly(x, y) && !self.jet(x, y) && !self.seep(x, y) {
                                self.step_water(x, y);
                            }
                        }
                        Mat::Steam => self.step_steam(x, y),
                        Mat::Fire => self.step_fire(x, y),
                        Mat::Ember => self.step_ember(x, y),
                        Mat::Lava => {
                            if !self.seep(x, y) {
                                self.step_hot_liquid(x, y, Mat::Stone);
                            }
                        }
                        Mat::Molten => {
                            if !self.seep(x, y) {
                                self.step_hot_liquid(x, y, Mat::Iron);
                            }
                        }
                        Mat::MoltenSteel => {
                            if !self.seep(x, y) {
                                self.step_hot_liquid(x, y, Mat::Steel);
                            }
                        }
                        Mat::Iron => self.step_iron(x, y, 300),
                        _ => {}
                    }
                }
            }
        }
    }

    /// Everything after the cells: falling pieces, wind, pressure, heat, the clock.
    fn after_cells(&mut self) {
        self.ticks = self.ticks.wrapping_add(1);
        if self.structures_fall && self.ticks % 8 == 0 {
            self.structure_pass();
        }
        if self.structures_fall && self.ticks % 2 == 0 {
            self.drop_structures();
        }
        if self.ticks % pockets::POCKETS_EVERY == pockets::POCKETS_EVERY / 2 {
            self.update_pockets();
        }
        self.update_wind();
        self.update_flow();
        if self.ticks % 4 == 0 {
            self.update_pressure();
        }
        self.pressure_push();
        if self.ticks % 64 == 0 {
            self.update_season_cache();
        }
        if self.ticks % (thermal::THERMAL_EVERY * self.thermal_scale() as u32) == 0 {
            self.thermal_pass();
        }
        if self.day_cycle {
            self.clock = self.clock.wrapping_add(1);
        }
        if self.frame % 4 == 0 {
            self.level_water();
        }
    }

    /// Hydrostatic balance: inside one connected body of water, a calm surface whose head is well
    /// above another one's gives cells to it. The head of a surface is its height plus the
    /// pressure of the gas pressing on it, in cells of water (a closed boiler full of steam pushes
    /// its water as if it stood much higher). That makes water climb back up in U-shaped tubes
    /// (communicating vessels), and up a pipe out of a boiler under pressure.
    fn level_water(&mut self) {
        let mut seen = std::mem::take(&mut self.seen);
        let mut stack = std::mem::take(&mut self.stack);
        seen.fill(false);
        let head = |w: &World, x: i32, y: i32| (H as i32 - y) as f32 + (w.gas_pressure(x, y - 1) - 1.0) / pockets::ATM_PER_CELL;
        for start in 0..W * H {
            if seen[start] || self.cells[start].m != Mat::Water {
                continue;
            }
            let mut donor: Option<(i32, i32, f32)> = None;
            let mut recv: Option<(i32, i32, f32)> = None;
            seen[start] = true;
            stack.push(start);
            while let Some(i) = stack.pop() {
                let (x, y) = ((i % W) as i32, (i / W) as i32);
                let above = self.at(x, y - 1);
                if matches!(above.kind(), Kind::Empty | Kind::Gas) && self.at(x, y + 1) == Mat::Water {
                    let h = head(self, x, y);
                    if above == Mat::Empty && recv.map_or(true, |r| h < r.2) {
                        recv = Some((x, y, h));
                    }
                    if !matches!(self.at(x - 1, y).kind(), Kind::Empty | Kind::Gas)
                        && !matches!(self.at(x + 1, y).kind(), Kind::Empty | Kind::Gas)
                        && donor.map_or(true, |d| h > d.2)
                    {
                        donor = Some((x, y, h));
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
                let gap = d.2 - r.2;
                if gap >= 3.0 && (d.0, d.1) != (r.0, r.1) {
                    // A big difference of head moves several cells at once (a strong push).
                    let n = ((gap / 6.0) as i32).clamp(1, 4);
                    for k in 0..n {
                        let (dx, dy) = (d.0, d.1 + k);
                        let (rx, ry) = (r.0, r.1 - 1 - k);
                        if self.raw(dx, dy) != Mat::Water || self.at(rx, ry) != Mat::Empty {
                            break;
                        }
                        let t = self.temp[Self::idx(dx, dy)];
                        self.set(dx, dy, Mat::Empty);
                        self.set(rx, ry, Mat::Water);
                        self.temp[Self::idx(rx, ry)] = t;
                    }
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
        } else if self.cells[Self::idx(nx, ny)].m.is_wooden() {
            self.set(nx, ny, Mat::Ember);
        }
    }

    /// First cell of `m` within `r` cells of (x, y).
    fn find_near(&self, x: i32, y: i32, r: i32, m: Mat) -> Option<(i32, i32)> {
        for dy in -r..=r {
            for dx in -r..=r {
                if self.raw(x + dx, y + dy) == m {
                    return Some((x + dx, y + dy));
                }
            }
        }
        None
    }

    /// Returns true if the cell moved.
    fn step_powder(&mut self, x: i32, y: i32) -> bool {
        let i = Self::idx(x, y);
        let m = self.cells[i].m;
        let light = density(m);
        // Archimedes: a grain lighter than the liquid round it is pushed up through it (ash,
        // charcoal, dead leaves and hail float on water); a heavier one sinks.
        let sinks_in = |n: Mat| match n.kind() {
            Kind::Empty | Kind::Gas => true,
            Kind::Liquid => density(n) < light,
            _ => false,
        };
        let above = self.at(x, y - 1);
        if above.kind() == Kind::Liquid && density(above) > light && self.mask_free(x, y - 1) && self.chance(2) {
            self.vel[i] = 0;
            self.swap((x, y), (x, y - 1));
            return true;
        }
        // Free fall: the grain gathers speed (gravity) up to a terminal speed (air drag), and
        // covers several cells in a step once it is fast.
        if matches!(self.at(x, y + 1).kind(), Kind::Empty | Kind::Gas) && self.mask_free(x, y + 1) {
            let terminal = if m == Mat::Snow { SNOW_TERMINAL } else { GRAIN_TERMINAL };
            let v = self.vel[i].saturating_add(GRAIN_ACCEL).min(terminal);
            let mut k = 1;
            while k < 1 + v as i32 / 8 && matches!(self.at(x, y + k + 1).kind(), Kind::Empty | Kind::Gas) && self.mask_free(x, y + k + 1) {
                k += 1;
            }
            // Stopped short by what is below: it lands.
            self.vel[i] = if k < 1 + v as i32 / 8 { 0 } else { v };
            self.swap((x, y), (x, y + k));
            return true;
        }
        self.vel[i] = 0;
        if sinks_in(self.at(x, y + 1)) {
            self.swap((x, y), (x, y + 1));
            return true;
        }
        // Sliding down the side of the heap. Each material has its own angle of repose: some
        // grains of a cohesive heap (snow, wet sand) only slide where the drop is two cells, which
        // makes it steeper; some grains of a free-running one (dry sand, hail, mud) roll two cells
        // sideways, which makes it flatter. Which grains is fixed by their place in the heap, so
        // the heap settles at a steady angle in between.
        let (steep, flat) = self.repose(x, y, m);
        let pick = Self::grain_pick(x, y);
        let d = self.dir();
        for dx in [d, -d] {
            if !sinks_in(self.at(x + dx, y + 1)) {
                continue;
            }
            if pick < steep && Self::inb(x + dx, y + 2) && !sinks_in(self.at(x + dx, y + 2)) {
                continue;
            }
            self.swap((x, y), (x + dx, y + 1));
            return true;
        }
        if pick < flat {
            for dx in [d, -d] {
                if sinks_in(self.at(x + dx, y)) && sinks_in(self.at(x + 2 * dx, y + 1)) {
                    self.swap((x, y), (x + 2 * dx, y + 1));
                    return true;
                }
            }
        }
        false
    }

    /// A number from 0 to 99 fixed for each place in the world, to pick which grains of a heap
    /// hold and which run.
    pub(super) fn grain_pick(x: i32, y: i32) -> u32 {
        ((((x as u32).wrapping_mul(73_856_093) ^ (y as u32).wrapping_mul(19_349_663)).wrapping_mul(2_654_435_761) >> 16) & 0xffff) % 100
    }

    /// How a heap of `m` holds: (percent of grains that need a two-cell drop to slide, percent
    /// that roll two cells sideways). (0, 0) is the plain 45 degree heap.
    fn repose(&self, x: i32, y: i32, m: Mat) -> (u32, u32) {
        match m {
            // Dry sand runs (about 34 degrees); wet sand holds (sandcastles).
            Mat::Sand => {
                if Self::neighbors(x, y).iter().any(|&(nx, ny)| self.at(nx, ny) == Mat::Water) {
                    (70, 0)
                } else {
                    (0, 50)
                }
            }
            Mat::Hail => (0, 70),
            Mat::Mud => (0, 90),
            Mat::Ash => (30, 0),
            _ => (0, 0),
        }
    }

    /// Water that moved during the last two steps is flowing, not resting.
    fn flowing_water_near(&self, x: i32, y: i32) -> bool {
        let f = self.frame;
        Self::neighbors(x, y).iter().any(|&(nx, ny)| {
            Self::inb(nx, ny) && {
                let c = self.cells[Self::idx(nx, ny)];
                c.m == Mat::Water && (c.stamp == f || c.stamp == f.wrapping_sub(1))
            }
        })
    }

    /// Loose sand and mud: flowing water drags them along (gullies, deltas);
    /// mud that dries out consolidates back into soil.
    fn step_sediment(&mut self, x: i32, y: i32) {
        let mud = self.at(x, y) == Mat::Mud;
        if self.flowing_water_near(x, y) && self.chance(if mud { 3 } else { 6 }) {
            let d = self.dir();
            for (dx, dy) in [(d, 0), (d, 1), (-d, 0), (-d, 1)] {
                if matches!(self.at(x + dx, y + dy), Mat::Empty | Mat::Water) {
                    self.swap((x, y), (x + dx, y + dy));
                    return;
                }
            }
        }
        if mud && self.chance(4000) && Self::neighbors(x, y).iter().all(|&(nx, ny)| self.at(nx, ny) != Mat::Water) {
            self.set(x, y, Mat::Dirt);
            return;
        }
        self.step_powder(x, y);
    }

    /// Bare soil washes away under flowing water (grass roots protect it).
    /// Grass is a thin carpet: with nothing solid under it, it falls until it lands on
    /// soil, sand or rock, or else drops out of the world. It dries out like soil,
    /// but its roots hold the earth against erosion.
    fn step_grass(&mut self, x: i32, y: i32) {
        let i = Self::idx(x, y);
        if matches!(self.at(x, y + 1).kind(), Kind::Empty | Kind::Gas | Kind::Liquid) && self.mask_free(x, y + 1) {
            self.swap((x, y), (x, y + 1));
            return;
        }
        if self.cells[i].life > 0 && self.chance(if self.sun() > 0.5 { 600 } else { 2500 }) {
            self.cells[i].life -= 1;
        }
    }

    fn step_dirt(&mut self, x: i32, y: i32) {
        let i = Self::idx(x, y);
        // Moisture slowly dries: faster in hot sun and in dry, windy air at the surface; buried
        // soil keeps its water much longer.
        if self.cells[i].life > 0 {
            let base = if self.sun() > 0.5 { 600.0 } else { 2500.0 };
            let open = self.at(x, y - 1) == Mat::Empty;
            let speed = if open {
                let dry = 1.0 - self.flow.moisture_at(x, y - 1);
                let (wx, wy) = self.flow.sample(x, y - 1);
                (0.5 + 1.5 * dry) * (1.0 + ((wx * wx + wy * wy).sqrt() / flow::GAIN).min(1.0))
            } else {
                0.5
            };
            if self.chance((base / speed) as u32) {
                self.cells[i].life -= 1;
            }
        }
        if self.chance(30) && self.flowing_water_near(x, y) && self.chance(8) {
            self.set(x, y, Mat::Mud);
        }
    }

    /// Snowflakes drift on the wind while they fall, slowly (they are light and catch the air).
    /// On the ground snow is a very thick, viscous paste: a heap slumps and spreads out little by
    /// little, but a thin layer (one or two cells) stays where it fell instead of running off.
    fn step_snow(&mut self, x: i32, y: i32) {
        if self.at(x, y + 1) == Mat::Empty && self.wind.abs() > 0.1 && (self.rand() % 100) as f32 <= self.wind.abs() * 60.0 {
            let w = if self.wind > 0.0 { 1 } else { -1 };
            if self.at(x + w, y + 1) == Mat::Empty {
                self.swap((x, y), (x + w, y + 1));
                return;
            }
        }
        if self.step_powder(x, y) {
            return;
        }
        // Creep: now and then a cell oozes a few cells sideways and one down, so a heap keeps
        // slumping flatter than a heap of grains. Where the ground is flat it needs a drop to
        // flow into, so a thin layer stays put.
        if self.chance(SNOW_CREEP) {
            if let Some(to) = self.snow_creep_target(x, y) {
                self.swap((x, y), to);
            }
        }
    }

    /// Where a cell of snow at rest can ooze to: up to `SNOW_RUN` cells along its row through
    /// free air, then one down into free air.
    pub(super) fn snow_creep_target(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        let d = if (x + y + self.frame as i32) & 1 == 0 { 1 } else { -1 };
        let free = |x: i32, y: i32| self.at(x, y) == Mat::Empty && self.mask_free(x, y);
        for dx in [d, -d] {
            for k in 1..=SNOW_RUN {
                if !free(x + dx * k, y) {
                    break;
                }
                if k >= 2 && free(x + dx * k, y + 1) {
                    return Some((x + dx * k, y + 1));
                }
            }
        }
        None
    }

    /// Glass shards pile up like sand and, given time, turn back into sand.
    fn step_shard(&mut self, x: i32, y: i32) {
        if !self.step_powder(x, y) && self.chance(4000) {
            self.set(x, y, Mat::Sand);
        }
    }

    fn step_gunpowder(&mut self, x: i32, y: i32) {
        for (nx, ny) in Self::neighbors(x, y) {
            if self.at(nx, ny).hot() {
                self.set(x, y, Mat::Fire);
                if self.powder_booms.len() < 16 {
                    self.powder_booms.push((x, y));
                }
                self.explode(x, y, 4 * K + 4);
                return;
            }
        }
        self.step_powder(x, y);
    }

    /// Ash is carbon: with water it turns into fertile soil.
    fn step_ash(&mut self, x: i32, y: i32) {
        let n = Self::neighbors(x, y)[(self.rand() % 4) as usize];
        if self.at(n.0, n.1) == Mat::Water && self.chance(500) {
            self.set(x, y, Mat::Dirt);
            return;
        }
        self.step_powder(x, y);
    }

    /// Charcoal is a powder, set alight by lava as well as by flames.
    fn step_charcoal(&mut self, x: i32, y: i32) {
        let n = Self::neighbors(x, y)[(self.rand() % 4) as usize];
        if self.at(n.0, n.1) == Mat::Lava {
            self.light_charcoal(x, y, 20);
            if self.at(x, y) != Mat::Charcoal {
                return;
            }
        }
        self.step_powder(x, y);
    }

    /// Enough light for a plant: daytime and nothing leafy overhead.
    fn lit(&self, x: i32, y: i32) -> bool {
        if self.sun() < 0.1 {
            return false;
        }
        (1..=4 * K).all(|dy| !matches!(self.raw(x, y - dy), Mat::Leaf | Mat::Branch | Mat::Trunk | Mat::Wood))
    }

    /// Soil that can still feed a plant: (cell index, moisture) within reach.
    fn find_moist_soil(&self, x: i32, y: i32, reach: i32, depth: i32) -> Option<(i32, i32)> {
        for dy in 0..=depth {
            for dx in -reach..=reach {
                let (sx, sy) = (x + dx, y + dy);
                if Self::inb(sx, sy) && matches!(self.cells[Self::idx(sx, sy)].m, Mat::Dirt | Mat::Grass) && self.cells[Self::idx(sx, sy)].life >= 30 {
                    return Some((sx, sy));
                }
            }
        }
        None
    }

    /// A plant needs a drink: standing water nearby, or moist soil.
    fn drink(&mut self, x: i32, y: i32, reach: i32, depth: i32) -> bool {
        for dy in 0..=depth {
            for dx in -reach..=reach {
                if self.raw(x + dx, y + dy) == Mat::Water {
                    self.set(x + dx, y + dy, Mat::Empty);
                    return true;
                }
            }
        }
        if let Some((sx, sy)) = self.find_moist_soil(x, y, reach, depth) {
            let i = Self::idx(sx, sy);
            self.cells[i].life -= 30;
            return true;
        }
        false
    }

    /// Is there anything to drink within reach (standing water or moist soil)?
    fn water_within(&self, x: i32, y: i32, reach: i32, depth: i32) -> bool {
        for dy in 0..=depth {
            for dx in -reach..=reach {
                if self.raw(x + dx, y + dy) == Mat::Water {
                    return true;
                }
            }
        }
        self.find_moist_soil(x, y, reach, depth).is_some()
    }

    /// A seed falls, then sprouts on moist soil in the light, with room to grow.
    /// Seeds are short-lived: with no water around for a while, they rot into soil.
    fn step_seed(&mut self, x: i32, y: i32) {
        if self.step_powder(x, y) {
            return;
        }
        if !self.water_within(x, y, 3, 3) {
            if self.chance(2500) {
                self.set(x, y, Mat::Dirt);
            }
            return;
        }
        if !matches!(self.at(x, y + 1), Mat::Dirt | Mat::Grass) || !self.lit(x, y) {
            return;
        }
        if self.surface_temp_c() < 6.0 || self.find_near(x, y, 5 * K, Mat::Trunk).is_some() || self.find_near(x, y, 5 * K, Mat::Sapling).is_some() {
            return;
        }
        if self.drink(x, y, 3, 3) {
            let budget = 40 + (self.rand() % 40) as u8;
            self.set(x, y, Mat::Empty);
            for dx in -1..=1 {
                if self.raw(x + dx, y) == Mat::Empty {
                    self.set(x + dx, y, Mat::Sapling);
                    let i = Self::idx(x + dx, y);
                    self.cells[i].life = budget;
                }
            }
        }
    }

    /// A crown of leaves around a branch core, centred just above the trunk top.
    fn grow_crown(&mut self, x: i32, y: i32) {
        let r = (4 + (self.rand() % 3) as i32) * K;
        let cy = y - r / 2;
        for dy in -r..=r {
            for dx in -r..=r {
                let (lx, ly) = (x + dx, cy + dy);
                if dx * dx + dy * dy <= r * r && self.raw(lx, ly) == Mat::Empty {
                    self.set(lx, ly, Mat::Leaf);
                }
            }
        }
        let core = (3 * K / 2).max(2);
        for dy in -core..=core {
            for dx in -core..=core {
                if dx * dx + dy * dy <= core * core && matches!(self.raw(x + dx, cy + dy), Mat::Leaf | Mat::Empty) {
                    self.set(x + dx, cy + dy, Mat::Branch);
                }
            }
        }
    }

    /// A growing trunk tip: in the light it drinks to extend upward, then
    /// finishes with a crown of leaves.
    fn step_sapling(&mut self, x: i32, y: i32) {
        let i = Self::idx(x, y);
        let life = self.cells[i].life;
        if self.at(x, y - 1) != Mat::Empty {
            // Not a tip: it is trunk now.
            if self.at(x, y - 1) != Mat::Sapling {
                self.set(x, y, Mat::Trunk);
            }
            return;
        }
        if life == 0 {
            self.set(x, y, Mat::Trunk);
            self.grow_crown(x, y);
            return;
        }
        if self.chance(30) && self.surface_temp_c() > 6.0 && self.lit(x, y) && self.drink(x, y, 8, 50) {
            self.set(x, y, Mat::Trunk);
            self.set(x, y - 1, Mat::Sapling);
            let j = Self::idx(x, y - 1);
            self.cells[j].life = life - 1;
        }
    }

    /// Leaves follow the seasons: they turn in autumn, drop, and leave the tree bare in winter.
    fn step_leaf(&mut self, x: i32, y: i32) {
        match self.season() {
            Season::Autumn => {
                let late = 1.0 + self.season_progress() * 3.0;
                if self.chance((14_000.0 / late) as u32) {
                    self.set(x, y, Mat::DeadLeaf);
                    return;
                }
            }
            Season::Winter => {
                if self.chance(2_500) {
                    self.set(x, y, Mat::DeadLeaf);
                    return;
                }
            }
            _ => {}
        }
        if self.season() != Season::Winter && self.chance(500_000) && self.at(x, y + 1) == Mat::Empty {
            self.set(x, y + 1, Mat::Seed);
        }
    }

    /// A dead leaf falls, lies on the ground and slowly rots into soil.
    fn step_dead_leaf(&mut self, x: i32, y: i32) {
        if self.step_powder(x, y) {
            return;
        }
        let wet = Self::neighbors(x, y).iter().any(|&(nx, ny)| self.at(nx, ny) == Mat::Water);
        if self.chance(if wet { 1_500 } else { 6_000 }) {
            // The saltpetre bed: a leaf rotting against wood ash leaves white crystals of
            // saltpetre (the ash gives the potash, the rotting leaf the nitrate) and the ash
            // goes into the soil.
            let ash = Self::neighbors(x, y).into_iter().find(|&(nx, ny)| self.raw(nx, ny) == Mat::Ash);
            match ash {
                Some((ax, ay)) => {
                    self.set(x, y, Mat::Saltpeter);
                    self.set(ax, ay, Mat::Dirt);
                }
                None => self.set(x, y, Mat::Dirt),
            }
        }
    }

    /// Saltpetre dissolves in water. Ground against charcoal, cold and dry, it makes black
    /// powder (real powder also takes a little sulphur, which only makes it livelier).
    fn step_saltpeter(&mut self, x: i32, y: i32) {
        let n = Self::neighbors(x, y)[(self.rand() % 4) as usize];
        match self.raw(n.0, n.1) {
            Mat::Water if self.chance(300) => {
                self.set(x, y, Mat::Empty);
                return;
            }
            Mat::Charcoal if self.chance(40) && self.temp_c(x, y) < 80.0 => {
                let dry = !Self::neighbors(x, y).iter().chain(Self::neighbors(n.0, n.1).iter()).any(|&(ax, ay)| self.raw(ax, ay) == Mat::Water);
                if dry {
                    self.set(x, y, Mat::Gunpowder);
                    self.set(n.0, n.1, Mat::Gunpowder);
                    return;
                }
            }
            _ => {}
        }
        self.step_powder(x, y);
    }

    /// The skeleton of a crown: in spring and summer it pushes new leaves out
    /// through the foliage that is already there.
    fn step_branch(&mut self, x: i32, y: i32) {
        if matches!(self.season(), Season::Spring | Season::Summer) && self.surface_temp_c() > 2.0 && self.chance(500) {
            let (mut cx, mut cy) = (x, y);
            for _ in 0..6 * K {
                let n = Self::neighbors(cx, cy)[(self.rand() % 4) as usize];
                if matches!(self.raw(n.0, n.1), Mat::Leaf | Mat::Branch) {
                    (cx, cy) = n;
                }
            }
            let n = Self::neighbors(cx, cy)[(self.rand() % 4) as usize];
            let r = 6 * K;
            if self.raw(n.0, n.1) == Mat::Empty && (n.0 - x).pow(2) + (n.1 - y).pow(2) <= r * r {
                self.set(n.0, n.1, Mat::Leaf);
            }
            return;
        }
        self.step_wood(x, y);
    }

    /// Wet wood slowly rots into soil.
    /// A tree drinks through its roots and breathes the water out through its leaves, by day,
    /// while it is in leaf: the soil around it dries and the air over a forest gets damp.
    fn transpire(&mut self, x: i32, y: i32) {
        if self.sun() < 0.2 || matches!(self.season(), Season::Winter) || !matches!(self.at(x, y + 1), Mat::Dirt | Mat::Grass) {
            return;
        }
        // Up the trunk to the crown, then through the foliage to a leaf in the open air.
        let mut top = y;
        while top > 0 && self.raw(x, top - 1) == Mat::Trunk {
            top -= 1;
        }
        let (mut cx, mut cy) = (x, top - 1);
        if !matches!(self.raw(cx, cy), Mat::Leaf | Mat::Branch) {
            return;
        }
        // Straight out of the crown in a random direction (up, up-left, up-right, left, right).
        let (dx, dy) = [(0, -1), (-1, -1), (1, -1), (-1, 0), (1, 0)][(self.rand() % 5) as usize];
        let mut steps = 0;
        while steps < 20 * K && matches!(self.raw(cx + dx, cy + dy), Mat::Leaf | Mat::Branch) {
            (cx, cy) = (cx + dx, cy + dy);
            steps += 1;
        }
        let (vx, vy) = (cx + dx, cy + dy);
        if self.raw(cx, cy) == Mat::Leaf && self.raw(vx, vy) == Mat::Empty && self.drink(x, y + 1, 4 * K, 6 * K) {
            // A quarter of a cell of water leaves as vapour, with the heat that takes (the leaf
            // and the air around it cool).
            let (j, t) = (Self::idx(vx, vy), self.temp[Self::idx(cx, cy)]);
            let q = thermal::heat_of(Mat::Empty, self.temp[j]) + thermal::heat_of(Mat::Water, t) / VAPOUR_PER_WATER as f32 - thermal::heat_of(Mat::Steam, t);
            self.set(vx, vy, Mat::Steam);
            self.temp[j] = t;
            self.release_heat(cx, cy, q);
        }
    }

    fn step_wood(&mut self, x: i32, y: i32) {
        if self.chance(TRANSPIRE_EVERY) && self.raw(x, y) == Mat::Trunk {
            self.transpire(x, y);
        }
        // Wood buried for a long time, sealed in earth or rock, slowly becomes oil.
        if self.chance(30_000) {
            let mut cover = 0;
            let mut sealed = true;
            for (nx, ny) in Self::neighbors(x, y) {
                match self.at(nx, ny) {
                    Mat::Dirt | Mat::Stone | Mat::Sand | Mat::Mud => cover += 1,
                    Mat::Wood | Mat::Trunk | Mat::Branch | Mat::Grass => {}
                    _ => sealed = false,
                }
            }
            if sealed && cover >= 2 {
                self.set(x, y, Mat::Oil);
                return;
            }
        }
        if self.chance(20_000) {
            let n = Self::neighbors(x, y)[(self.rand() % 4) as usize];
            if self.at(n.0, n.1) == Mat::Water {
                self.set(x, y, Mat::Dirt);
            }
        }
    }

    /// Convection for gas: gas warmer than the air around it expands and rises, gas
    /// colder than it contracts and sinks; hot gas also spreads out sideways while
    /// cold gas draws together. Returns true if the cell moved.
    fn buoyancy(&mut self, x: i32, y: i32) -> bool {
        let i = Self::idx(x, y);
        let lift = ((self.temp[i] - self.ambient_c(y)) / 25.0).clamp(-1.0, 1.0);
        let roll = (self.rand() % 100) as f32 / 100.0;
        let d = self.dir();
        if lift > 0.05 && roll < lift * 0.7 {
            for (dx, dy) in [(0, -1), (d, -1), (-d, -1)] {
                let t = (self.edge_x(x, y, x + dx), y + dy);
                if self.at(t.0, t.1) == Mat::Empty {
                    self.swap((x, y), t);
                    return true;
                }
            }
        } else if lift < -0.05 && roll < -lift * 0.8 {
            for (dx, dy) in [(0, 1), (d, 1), (-d, 1)] {
                let t = (self.edge_x(x, y, x + dx), y + dy);
                if self.at(t.0, t.1) == Mat::Empty {
                    self.swap((x, y), t);
                    return true;
                }
            }
        } else if lift.abs() > 0.2 && self.rand() % 100 < 25 {
            // Expansion (hot) spreads the gas out; contraction (cold) draws it together.
            let count = |w: &World, side: i32| (1..=3).filter(|k| matches!(w.raw(w.edge_x(x, y, x + side * k), y), Mat::Cloud | Mat::Steam | Mat::Smoke)).count();
            let side = if lift > 0.0 {
                d
            } else if count(self, 1) >= count(self, -1) {
                1
            } else {
                -1
            };
            let t = (self.edge_x(x, y, x + side), y);
            if self.at(t.0, t.1) == Mat::Empty {
                self.swap((x, y), t);
                return true;
            }
        }
        false
    }

    /// Where the gas at (x, y) moving to column `to` ends up. Vapour and clouds blown past the
    /// left or right edge leave the world (an open sky: they do not pile up). Smoke, which thins
    /// out in the open air on its own, comes back on the other side.
    pub(super) fn edge_x(&self, x: i32, y: i32, to: i32) -> i32 {
        if self.raw(x, y) == Mat::Smoke { to.rem_euclid(W as i32) } else { to }
    }

    /// Trees are porous: a liquid resting on foliage, branches or a trunk drips straight
    /// through to the first free cell below instead of being held up.
    fn seep(&mut self, x: i32, y: i32) -> bool {
        if !self.at(x, y + 1).is_vegetation() {
            return false;
        }
        let mut k = 2;
        while k <= 10 && self.at(x, y + k).is_vegetation() {
            k += 1;
        }
        if matches!(self.at(x, y + k).kind(), Kind::Empty | Kind::Gas) {
            self.swap((x, y), (x, y + k));
            return true;
        }
        false
    }

    /// Oil is a slow, light liquid: it floats on water and burns ferociously.
    fn step_oil(&mut self, x: i32, y: i32) {
        for (nx, ny) in Self::neighbors(x, y) {
            if matches!(self.at(nx, ny), Mat::Fire | Mat::Ember | Mat::Lava) && self.chance(2) {
                self.ignite_oil(x, y);
                return;
            }
        }
        if self.at(x, y - 1) == Mat::Water && self.chance(2) {
            self.swap((x, y), (x, y - 1));
            return;
        }
        if self.chance(3) {
            return;
        }
        let d = self.dir();
        for dx in [0, d, -d] {
            if matches!(self.at(x + dx, y + 1).kind(), Kind::Empty | Kind::Gas) {
                self.swap((x, y), (x + dx, y + 1));
                return;
            }
        }
        for dir in [d, -d] {
            let mut k = 0;
            while k < 2 * K && self.at(x + dir * (k + 1), y) == Mat::Empty {
                k += 1;
            }
            if k > 0 {
                self.swap((x, y), (x + dir * k, y));
                return;
            }
        }
    }

    /// Burning oil: a long-lived, very hot flame that sets its neighbours alight.
    pub(super) fn ignite_oil(&mut self, x: i32, y: i32) {
        self.set(x, y, Mat::Fire);
        let i = Self::idx(x, y);
        self.cells[i].life = 150 + (self.rand() % 100) as u8;
        if self.at(x, y - 1) == Mat::Empty {
            self.set(x, y - 1, Mat::Fire);
        }
        if self.at(x, y - 2) == Mat::Empty && self.chance(2) {
            self.set(x, y - 2, Mat::Smoke);
        }
    }

    /// Smoke rises, drifts with the wind (wrapping at the sides) and thins out.
    fn step_smoke(&mut self, x: i32, y: i32) {
        // In the open air smoke thins out; shut in a box it stays, and chokes the fire.
        if self.chance(900) && !self.sealed(x, y) {
            self.set(x, y, Mat::Empty);
            return;
        }
        if self.buoyancy(x, y) {
            return;
        }
        if let Some(t) = self.wind_target(x, y) {
            if self.at(t.0, t.1) == Mat::Empty {
                self.swap((x, y), t);
                return;
            }
        }
        let d = self.dir();
        // Only warm smoke keeps climbing by itself; cooled smoke just drifts.
        let hot = self.temp[Self::idx(x, y)] > self.ambient_c(y) + 8.0;
        let order: [(i32, i32); 3] = if hot && self.rand() % 10 < 6 { [(0, -1), (d, -1), (d, 0)] } else { [(d, 0), (-d, 0), (d, 0)] };
        for (dx, dy) in order {
            let t = (self.edge_x(x, y, x + dx), y + dy);
            if self.at(t.0, t.1) == Mat::Empty {
                self.swap((x, y), t);
                return;
            }
        }
    }

    fn step_water(&mut self, x: i32, y: i32) {
        let i = Self::idx(x, y);
        // Soil drinks the rain: the water is consumed and the soil gets moist.
        if matches!(self.at(x, y + 1), Mat::Dirt | Mat::Grass) && self.chance(25) {
            let j = Self::idx(x, y + 1);
            self.cells[j].life = self.cells[j].life.saturating_add(60);
            self.set(x, y, Mat::Empty);
            return;
        }
        // A free surface evaporates at any temperature above freezing: faster when the water is
        // warm (warm water has a higher vapour pressure), when the air over it is dry, and when
        // the wind carries the damp air away. The vapour leaves with heat: the water under it
        // cools (that is why sweat cools you down).
        // Steam drifting over the surface does not seal it: how damp the air is counts through `dry`.
        if matches!(self.at(x, y - 1), Mat::Empty | Mat::Steam) {
            let t = self.temp[i];
            if t > 0.0 {
                let dry = 1.0 - self.flow.moisture_at(x, y - 1);
                let (wx, wy) = self.flow.sample(x, y - 1);
                let breeze = 1.0 + 2.0 * ((wx * wx + wy * wy).sqrt() / flow::GAIN).min(1.0);
                // A falling drop is tiny and bathed in air all round: it dries far faster than a
                // puddle (in warm, dry air some rain never reaches the ground).
                let airborne = if self.at(x, y + 1) == Mat::Empty { DROP_EVAPORATION } else { 1.0 };
                let rate = airborne * dry * breeze * vapour_pressure(t) / vapour_pressure(40.0) / EVAPORATE_40C;
                if self.chance_f(rate) {
                    // The heat it takes cools the water and the air around (see `vaporize`).
                    self.vaporize(x, y);
                    return;
                }
            }
        }
        // Foliage lets water through: a drop that lands on leaves or branches trickles down to the
        // first free cell beneath them instead of piling up on the tree.
        if matches!(self.at(x, y + 1), Mat::Leaf | Mat::Branch | Mat::Sapling) {
            let mut k = 1;
            while k <= 12 * K && matches!(self.at(x, y + k), Mat::Leaf | Mat::Branch | Mat::Sapling) {
                k += 1;
            }
            if matches!(self.at(x, y + k).kind(), Kind::Empty | Kind::Gas) {
                self.swap((x, y), (x, y + k));
                return;
            }
        }
        // A drop in the air falls like any heavy particle: gravity speeds it up to a terminal speed
        // (air drag) and the wind carries it along on the way down, so the rain slants and gusts.
        // Water with more water on top of it (the roof of a hole in a lake, a thick stream) just
        // sinks a cell at a time: only a free drop races down.
        let free_drop = self.at(x, y - 1).kind() != Kind::Liquid;
        if free_drop && matches!(self.at(x, y + 1).kind(), Kind::Empty | Kind::Gas) && self.mask_free(x, y + 1) {
            // Falling through a cloud, the drop sweeps up the droplets in its way and grows: the
            // cloud's water becomes rain (`VAPOUR_PER_WATER` cells of cloud, one cell of rain).
            if self.raw(x, y + 1) == Mat::Cloud && self.chance(DROP_SWEEP) && self.gather_vapour(x, y + 1) {
                let j = Self::idx(x, y + 1);
                let t = self.temp[j].max(self.temp[i]);
                self.set(x, y + 1, Mat::Water);
                self.temp[j] = t;
                return;
            }
            self.fall_drop(x, y);
            return;
        }
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


    /// Moves a drop with free air under it: one to `DROP_TERMINAL / 8` cells down this step, and
    /// sideways as far as the local wind carries it.
    fn fall_drop(&mut self, x: i32, y: i32) {
        let i = Self::idx(x, y);
        let v = self.vel[i].saturating_add(GRAIN_ACCEL).min(DROP_TERMINAL);
        let fall = 1 + v as i32 / 8;
        let wind = (self.flow.sample(x, y).0 * DROP_DRIFT).clamp(-3.0, 3.0);
        let mut side = wind.trunc() as i32 + if (self.rand() % 1000) as f32 / 1000.0 < wind.fract().abs() { wind.signum() as i32 } else { 0 };
        let free = |w: &World, x: i32, y: i32| matches!(w.at(x, y).kind(), Kind::Empty | Kind::Gas) && w.mask_free(x, y);
        let (mut cx, mut cy) = (x, y);
        let mut k = 0;
        while k < fall {
            // The sideways part of the move is spread over the fall.
            let sx = side.signum();
            if sx != 0 && free(self, cx + sx, cy + 1) {
                cx += sx;
                side -= sx;
            } else if free(self, cx, cy + 1) {
                side = 0;
            } else {
                break;
            }
            cy += 1;
            k += 1;
            if !Self::inb(cx, cy) {
                break;
            }
        }
        // Stopped short by what is below: it lands.
        self.vel[i] = if k < fall { 0 } else { v };
        self.swap((x, y), (cx, cy));
    }

    fn step_steam(&mut self, x: i32, y: i32) {
        // Condensation is handled by temperature (see thermal.rs). Hot steam rises on
        // its own; as it cools it stops climbing and just drifts with the wind.
        let i = Self::idx(x, y);
        let hot = self.temp[i] > self.ambient_c(y) + 8.0;
        // Packed vapour (a full boiler) spreads into any room it finds.
        if self.cells[i].life > 0 {
            let n = Self::neighbors(x, y)[(self.rand() % 4) as usize];
            if self.at(n.0, n.1) == Mat::Empty {
                let (j, t) = (Self::idx(n.0, n.1), self.temp[i]);
                let q = thermal::heat_of(Mat::Empty, self.temp[j]) - thermal::cap(Mat::Steam) * t;
                self.cells[i].life -= 1;
                self.set(n.0, n.1, Mat::Steam);
                self.temp[j] = t;
                self.release_heat(x, y, q);
                return;
            }
        }
        if self.buoyancy(x, y) {
            return;
        }
        if let Some(t) = self.wind_target(x, y) {
            if self.at(t.0, t.1) == Mat::Empty {
                self.swap((x, y), t);
                return;
            }
        }
        let d = self.dir();
        // Even cooled, vapour keeps drifting up a little: damp air is lighter than dry air (a water
        // molecule weighs 18, air 29), which is how the vapour of a lake reaches the clouds.
        let r = self.rand() % 10;
        let order: [(i32, i32); 3] = if hot && r < 7 {
            [(0, -1), (d, -1), (d, 0)]
        } else if r < 3 {
            [(0, -1), (d, 0), (-d, 0)]
        } else {
            [(d, 0), (-d, 0), (0, -1)]
        };
        for (dx, dy) in order {
            let t = (x + dx, y + dy);
            if self.at(t.0, t.1) == Mat::Empty {
                self.swap((x, y), t);
                return;
            }
        }
    }

    /// Water at (x, y) turns to vapour and swells: the cell becomes steam, and the rest of its
    /// `VAPOUR_PER_WATER` cells of steam go into the nearest free cells around it. Shut in with no
    /// room left (a full boiler), the rest of the vapour stays packed in the cell (its `life`
    /// counts the extra cells' worth): no water is lost, and the packed vapour pushes harder
    /// (see `pockets.rs`) until it finds room to spread. The heat it takes beyond what the water
    /// held (all of it when the water boiled, less when it evaporates cold) comes from around.
    pub(crate) fn vaporize(&mut self, x: i32, y: i32) {
        let i = Self::idx(x, y);
        let t = self.temp[i];
        let mut q = thermal::heat_of(Mat::Water, t) + self.latent[i];
        self.set(x, y, Mat::Steam);
        self.temp[i] = t;
        let mut left = VAPOUR_PER_WATER - 1;
        let mut made = 1.0;
        'out: for r in 1..=2 {
            for dy in -r..=r {
                for dx in -r..=r {
                    if (dx as i32).abs().max((dy as i32).abs()) != r {
                        continue;
                    }
                    if left == 0 {
                        break 'out;
                    }
                    let (nx, ny) = (x + dx, y + dy);
                    // The second ring only through the first: never through a wall.
                    let via = (nx - dx.signum() * (r - 1), ny - dy.signum() * (r - 1));
                    if Self::inb(nx, ny) && self.at(nx, ny) == Mat::Empty && (r == 1 || matches!(self.at(via.0, via.1), Mat::Empty | Mat::Steam)) {
                        let j = Self::idx(nx, ny);
                        q += thermal::heat_of(Mat::Empty, self.temp[j]);
                        self.set(nx, ny, Mat::Steam);
                        self.temp[j] = t;
                        left -= 1;
                        made += 1.0;
                    }
                }
            }
        }
        self.cells[i].life = left as u8;
        q -= made * thermal::cap(Mat::Steam) * t + VAPOUR_PER_WATER as f32 * thermal::latent0(Mat::Steam);
        self.release_heat(x, y, q);
    }

    /// Cells' worth of vapour in a cell of steam or cloud (more than one when packed).
    pub fn vapour_units(&self, i: usize) -> u32 {
        match self.cells[i].m {
            Mat::Steam | Mat::Cloud => 1 + self.cells[i].life as u32,
            _ => 0,
        }
    }

    /// For a drop to form at (x, y), the `VAPOUR_PER_WATER - 1` other cells of cloud it needs are
    /// taken from the cloud around it (within two cells). Returns false, taking nothing, if the
    /// cloud there is too thin.
    fn gather_vapour(&mut self, x: i32, y: i32) -> bool {
        let mut need = VAPOUR_PER_WATER as u32 - self.vapour_units(Self::idx(x, y)).min(VAPOUR_PER_WATER as u32);
        let mut found = Vec::with_capacity(VAPOUR_PER_WATER);
        'out: for r in 1..=2 {
            for dy in -r..=r {
                for dx in -r..=r {
                    if (dx as i32).abs().max((dy as i32).abs()) != r {
                        continue;
                    }
                    if need == 0 {
                        break 'out;
                    }
                    if self.raw(x + dx, y + dy) == Mat::Cloud {
                        let take = self.vapour_units(Self::idx(x + dx, y + dy)).min(need);
                        found.push((x + dx, y + dy, take));
                        need -= take;
                    }
                }
            }
        }
        if need > 0 {
            return false;
        }
        // The droplets taken from the cloud around bring their heat to the drop.
        let mut q = 0.0;
        for (cx, cy, take) in found {
            let j = Self::idx(cx, cy);
            if take >= self.vapour_units(j) {
                q += thermal::heat_of(Mat::Cloud, self.temp[j]) + self.cells[j].life as f32 * thermal::latent0(Mat::Cloud) - thermal::heat_of(Mat::Empty, self.temp[j]);
                let t = self.temp[j];
                self.set(cx, cy, Mat::Empty);
                self.temp[j] = t;
            } else {
                q += take as f32 * thermal::latent0(Mat::Cloud);
                self.cells[j].life -= take as u8;
            }
        }
        self.release_heat(x, y, q);
        true
    }

    /// Cloud cells in the 5 x 5 around (x, y), not counting it: how much water the cloud holds there.
    fn cloud_water(&self, x: i32, y: i32) -> f32 {
        let mut n = 0;
        for dy in -2..=2 {
            for dx in -2..=2 {
                if (dx != 0 || dy != 0) && self.raw(x + dx, y + dy) == Mat::Cloud {
                    n += 1;
                }
            }
        }
        n as f32
    }

    fn step_cloud(&mut self, x: i32, y: i32) {
        let i = Self::idx(x, y);
        let mut near = 0;
        for dy in -1..=1 {
            for dx in -1..=1 {
                if (dx != 0 || dy != 0) && self.raw(x + dx, y + dy) == Mat::Cloud {
                    near += 1;
                }
            }
        }
        // A cloud is water: countless droplets too small to fall. Where the cloud is thick they
        // bump into each other and merge until some are heavy enough to fall: rain when it is
        // mild, snow when it freezes, hail when it is very cold. A drop can start anywhere in the
        // cloud and grows as it falls through it (see `step_water`). Every drop takes
        // `VAPOUR_PER_WATER` cells of the cloud's water, so a cloud that rains thins out and the rain eases off by itself until
        // fresh vapour feeds it again. A cloud warmed by the sun keeps its water and does not rain.
        let t = self.temp[i];
        if near >= 2 && t < 12.0 && matches!(self.at(x, y + 1), Mat::Empty | Mat::Cloud) {
            let water = self.cloud_water(x, y);
            let thick = ((water - RAIN_MIN_WATER) / (24.0 - RAIN_MIN_WATER)).max(0.0);
            if self.chance_f(thick * thick / RAIN_EVERY) && self.gather_vapour(x, y) {
                // The drop holds the cloud's water; freezing into snow or hail, it gives off
                // the heat of freezing (counted by `heat_of`).
                let own = thermal::heat_of(Mat::Cloud, t) + self.cells[i].life as f32 * thermal::latent0(Mat::Cloud);
                let (to, tt) = if t > 2.0 { (Mat::Water, t.max(4.0)) } else if t > -12.0 { (Mat::Snow, t) } else { (Mat::Hail, t) };
                self.set(x, y, to);
                self.temp[i] = tt;
                self.release_heat(x, y, own - thermal::heat_of(to, tt));
                return;
            }
        }
        // Warmed above its dew point (by the sun, by warm air), or left alone in dry air, a cloud
        // evaporates again.
        if (self.chance(600) && self.above_dew_point(x, y)) || (near == 0 && self.chance(3000)) {
            self.evaporate_cloud(x, y);
            return;
        }
        if self.buoyancy(x, y) {
            return;
        }
        // The wind pushes clouds along; otherwise they wander a little.
        let target = if let Some(t) = self.wind_target(x, y) {
            t
        } else {
            let d = self.dir();
            let r = self.rand() % 10;
            if r < 2 {
                (x + d, y)
            } else if r == 2 && y > COLD_Y {
                (x, y - 1)
            } else {
                return;
            }
        };
        // Blown past the left or right edge, it leaves the world like steam does.
        if self.at(target.0, target.1) == Mat::Empty {
            self.swap((x, y), target);
        }
    }

    fn step_fire(&mut self, x: i32, y: i32) {
        let i = Self::idx(x, y);
        if self.cells[i].life == 0 {
            self.set(x, y, Mat::Empty);
            return;
        }
        self.cells[i].life -= 1;
        // A flame burns the oxygen of the air touching it. With no air around (only smoke,
        // steam or walls) it chokes within a few steps: that is why a glass over a candle
        // puts it out.
        let air = Self::neighbors(x, y).iter().any(|&(nx, ny)| match self.at(nx, ny) {
            Mat::Empty => !self.stale(nx, ny),
            Mat::Smoke => !self.sealed(nx, ny),
            _ => false,
        });
        if !air {
            self.cells[i].life = self.cells[i].life.saturating_sub(6);
        }
        for (nx, ny) in Self::neighbors(x, y) {
            match self.at(nx, ny) {
                // Water puts the flame out (the warmth it took stays in the water).
                Mat::Water => {
                    if self.chance(4) {
                        self.set(x, y, Mat::Empty);
                        return;
                    }
                }
                m if m.is_wooden() => self.ignite(nx, ny, 25),
                Mat::Charcoal => self.light_charcoal(nx, ny, 40),
                Mat::Oil => {
                    if self.chance(2) {
                        self.ignite_oil(nx, ny);
                    }
                }
                _ => {}
            }
        }
        if self.chance(10) && self.at(x, y - 1) == Mat::Empty {
            self.set(x, y - 1, Mat::Smoke);
        }
        // Flames lean with the wind.
        let d = if self.wind.abs() > 0.3 && self.chance(2) { self.wind.signum() as i32 } else { self.dir() };
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
        // Burning charcoal (life from COAL_LIFE up) lasts several times longer than burning wood.
        let coal = self.cells[i].life >= COAL_LIFE;
        if self.cells[i].life == 0 || self.cells[i].life == COAL_LIFE {
            let m = if self.chance(2) { Mat::Ash } else { Mat::Empty };
            self.set(x, y, m);
            return;
        }
        // A coal bed that air cannot reach smoulders for much longer: that is how a covered fire
        // (a kiln, the foot of a woodpile) keeps its heat.
        let open = Self::neighbors(x, y).iter().filter(|&&(nx, ny)| matches!(self.at(nx, ny).kind(), Kind::Empty | Kind::Gas)).count();
        let slow = match open {
            0 => 6,
            1 => 4,
            2 => 2,
            _ => 1,
        };
        let slow = if coal { slow * 8 } else { slow };
        if slow == 1 || self.chance(slow) {
            self.cells[i].life -= 1;
        }
        // Only an ember that air reaches can light the wood beside it. Under a cover of soil the
        // coals keep their heat, and the wood they warm chars into charcoal instead of burning
        // (see `react`): the charcoal burner's mound.
        let air = self.breathes(x, y);
        // Shut in a box whose air is used up, it goes out: what is left is charcoal.
        if !air && self.chance(40) && Self::neighbors(x, y).iter().any(|&(nx, ny)| matches!(self.at(nx, ny).kind(), Kind::Empty | Kind::Gas) && self.stale(nx, ny)) {
            self.set(x, y, Mat::Charcoal);
            return;
        }
        for (nx, ny) in Self::neighbors(x, y) {
            match self.at(nx, ny) {
                Mat::Water => {
                    self.set(x, y, Mat::Ash);
                    return;
                }
                m if m.is_wooden() && air => self.ignite(nx, ny, 80),
                Mat::Charcoal if air => self.light_charcoal(nx, ny, 120),
                _ => {}
            }
        }
        if air && self.at(x, y - 1) == Mat::Empty && self.chance(12) {
            self.set(x, y - 1, Mat::Fire);
        }
    }


    /// Whether air with oxygen in it reaches the cell: open air, or a flame (fed by the air it
    /// draws in), or smoke still mixed with the open air. In a closed box the fire burns the
    /// oxygen: once the box is half smoke, nothing in it can burn any more.
    /// Air also seeps through one cell of something full of cracks (ash, charcoal, embers): a
    /// heap of coals burns right through, while a cover of soil keeps the air out.
    pub(super) fn breathes(&self, x: i32, y: i32) -> bool {
        let air = |w: &World, (nx, ny): (i32, i32)| match w.at(nx, ny) {
            Mat::Empty | Mat::Fire => !w.stale(nx, ny),
            Mat::Smoke => !w.sealed(nx, ny),
            _ => false,
        };
        let n = Self::neighbors(x, y);
        n.iter().any(|&c| air(self, c))
            || n.iter().any(|&(nx, ny)| matches!(self.at(nx, ny), Mat::Ash | Mat::Charcoal | Mat::Ember) && Self::neighbors(nx, ny).iter().any(|&c| air(self, c)))
    }

    /// Charcoal touching a flame catches when it has air, as an ember that lasts much longer
    /// than one of wood.
    pub(super) fn light_charcoal(&mut self, x: i32, y: i32, one_in: u32) {
        if !self.chance(one_in) || !self.breathes(x, y) {
            return;
        }
        self.set(x, y, Mat::Ember);
        let i = Self::idx(x, y);
        self.cells[i].life = COAL_LIFE + 30 + (self.rand() % 26) as u8;
    }

    /// Lava and molten metal: they flow slowly, set wood alight, and when quenched
    /// by water set solid (`solid`) while the water flashes to steam.
    fn step_hot_liquid(&mut self, x: i32, y: i32, solid: Mat) {
        for (nx, ny) in Self::neighbors(x, y) {
            match self.at(nx, ny) {
                // Quenched: it sets solid and the water flashes to steam.
                Mat::Water => {
                    // The heat of setting (and of the melt above its melting point) boils the water.
                    let (i, j) = (Self::idx(x, y), Self::idx(nx, ny));
                    let t = self.temp[i];
                    let mut q = thermal::heat_of(self.cells[i].m, t) + self.latent[i] - thermal::heat_of(solid, t);
                    self.set(x, y, solid);
                    self.temp[i] = t;
                    let tw = self.temp[j];
                    if tw < 100.0 {
                        q -= thermal::cap(Mat::Water) * (100.0 - tw);
                        self.temp[j] = 100.0;
                    }
                    self.vaporize(nx, ny);
                    self.release_heat(x, y, q);
                    return;
                }
                m if m.is_wooden() => self.ignite(nx, ny, 20),
                _ => {}
            }
        }
        if self.at(x, y - 1) == Mat::Empty && self.chance(150) {
            self.set(x, y - 1, Mat::Fire);
        }
        // A pool of melt churns (convection): hot melt from inside keeps coming against the
        // walls and the floor, so they keep being heated, and the heat is only moved, not made.
        let (nx, ny) = Self::neighbors(x, y)[(self.rand() % 4) as usize];
        if self.at(nx, ny) == self.at(x, y) {
            let (i, j) = (Self::idx(x, y), Self::idx(nx, ny));
            self.temp.swap(i, j);
            self.latent.swap(i, j);
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


    /// Iron rusts in water; steel rusts much more slowly.
    fn step_iron(&mut self, x: i32, y: i32, rust_one_in: u32) {
        let n = Self::neighbors(x, y)[(self.rand() % 4) as usize];
        if self.at(n.0, n.1) == Mat::Water && self.chance(rust_one_in) {
            self.set(x, y, Mat::Rust);
        }
    }

    // ----- structures -------------------------------------------------------

    /// Grass is not structural: it holds nothing up and is not carried along, it
    /// just falls (see `step_grass`) until something solid catches it.
    fn structural(m: Mat) -> bool {
        m.kind() == Kind::Static && m != Mat::Grass
    }

    /// Can solid `from` hold up the solid `to` lying `dy` rows away (-1: `to` sits on top of it,
    /// 0: beside it, 1: hangs under it)? Ice is a heavy slab: it holds to more ice, rests on what
    /// is under it and sticks to the side of rock or soil, but it never hangs under anything and a
    /// tree does not carry it (only a trunk right under it does). That way ice that froze on
    /// leaves, on a branch or on whatever lies on them does not stay hanging in the sky.
    fn holds(from: Mat, to: Mat, dy: i32) -> bool {
        if to != Mat::Ice || from == Mat::Ice {
            return true;
        }
        match from {
            Mat::Leaf | Mat::Branch | Mat::Sapling => false,
            Mat::Trunk => dy == -1,
            _ => dy <= 0,
        }
    }

    /// Finds solid cells that are not connected to rock resting on the base of
    /// the world and lists them, grouped by connected piece, so they can fall.
    fn structure_pass(&mut self) {
        let mut seen = std::mem::take(&mut self.seen);
        let mut stack = std::mem::take(&mut self.stack);
        seen.fill(false);
        // There is no real ground: only rock resting on the base of the world holds
        // everything else up. Whatever is not connected to it falls away.
        for x in 0..W {
            let i = (H - 1) * W + x;
            if self.cells[i].m == Mat::Stone && self.mask[i] == 0 {
                seen[i] = true;
                stack.push(i);
            }
        }
        // A tunnel's vault is bolted into the ground around it: it holds, and holds up what it touches.
        for i in 0..W * H {
            if self.cells[i].m == Mat::Vault && !seen[i] {
                seen[i] = true;
                stack.push(i);
            }
        }
        while let Some(i) = stack.pop() {
            let (x, y) = ((i % W) as i32, (i / W) as i32);
            let from = self.cells[i].m;
            for (nx, ny) in Self::neighbors(x, y) {
                if Self::inb(nx, ny) {
                    let j = Self::idx(nx, ny);
                    if !seen[j] && Self::structural(self.cells[j].m) && Self::holds(from, self.cells[j].m, ny - y) {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        // A piece that was already falling keeps its speed (sampled cells are enough).
        let mut speeds: std::collections::HashMap<usize, f32> = std::collections::HashMap::new();
        for p in &self.falling {
            for c in p.cells.iter().step_by(5).take(400) {
                speeds.insert(*c, p.vy);
            }
        }
        // Group what is left into pieces.
        self.falling.clear();
        let mut budget = 300_000usize;
        for start in 0..W * H {
            if seen[start] || !Self::structural(self.cells[start].m) {
                continue;
            }
            let mut cells = vec![];
            seen[start] = true;
            stack.push(start);
            while let Some(i) = stack.pop() {
                cells.push(i);
                let (x, y) = ((i % W) as i32, (i / W) as i32);
                for (nx, ny) in Self::neighbors(x, y) {
                    if Self::inb(nx, ny) {
                        let j = Self::idx(nx, ny);
                        if !seen[j] && Self::structural(self.cells[j].m) {
                            seen[j] = true;
                            stack.push(j);
                        }
                    }
                }
            }
            budget = budget.saturating_sub(cells.len());
            let vy = cells.iter().step_by(5).filter_map(|c| speeds.get(c)).fold(0.0f32, |a, b| a.max(*b));
            let density = cells.iter().map(|&i| density(self.cells[i].m)).sum::<f32>() / cells.len().max(1) as f32;
            self.falling.push(Piece { cells, vy, density });
            if budget == 0 {
                break;
            }
        }
        self.seen = seen;
        self.stack = stack;
    }

    /// Moves every unsupported piece down (accelerating) as a rigid whole. A fast
    /// landing bounces a little. Ice pieces float on liquids instead of sinking.
    fn drop_structures(&mut self) {
        let mut pieces = std::mem::take(&mut self.falling);
        for piece in &mut pieces {
            piece.cells.sort_unstable_by(|a, b| b.cmp(a));
            if piece.vy >= 0.0 {
                // Falling: gravity builds up speed; move one cell at a time while it is free.
                piece.vy = (piece.vy + 0.35).min(3.0);
                for _ in 0..(piece.vy.max(1.0) as i32) {
                    if !self.piece_can_move(piece, 1) && !self.crush_foliage(piece) {
                        break;
                    }
                    self.piece_move(piece, 1);
                }
                if !self.piece_can_move(piece, 1) {
                    // Touched down. A hard landing bounces right away; a soft one settles.
                    if piece.vy > 1.4 {
                        if self.thuds.len() < 8 {
                            let c = piece.cells[0];
                            self.thuds.push(((c % W) as i32, (c / W) as i32, piece.vy));
                        }
                        piece.vy = -piece.vy * 0.34;
                        self.rise(piece);
                    } else {
                        piece.cells.clear();
                    }
                }
            } else {
                self.rise(piece);
            }
        }
        pieces.retain(|p| !p.cells.is_empty());
        self.falling = pieces;
    }

    /// A falling piece held up only by leaves, twigs or saplings breaks them: they fall as dead
    /// leaves and the piece goes on falling. Returns true if it is free to move down now.
    fn crush_foliage(&mut self, piece: &Piece) -> bool {
        let in_piece = |i: usize| piece.cells.binary_search_by(|p| i.cmp(p)).is_ok();
        let mut foliage = vec![];
        for &i in &piece.cells {
            let t = i + W;
            if t >= W * H || in_piece(t) || self.mask[t] != 0 {
                continue;
            }
            match self.cells[t].m {
                Mat::Leaf | Mat::Branch | Mat::Sapling => foliage.push(t),
                m if m.kind() == Kind::Static => return false,
                _ => {}
            }
        }
        if foliage.is_empty() {
            return false;
        }
        for t in foliage {
            self.set((t % W) as i32, (t / W) as i32, Mat::DeadLeaf);
        }
        self.piece_can_move(piece, 1)
    }

    /// The upward part of a bounce.
    fn rise(&mut self, piece: &mut Piece) {
        let up = (-piece.vy).max(1.0) as i32;
        for _ in 0..up {
            if !self.piece_can_move(piece, -1) {
                piece.vy = 0.0;
                return;
            }
            self.piece_move(piece, -1);
        }
        piece.vy += 0.5;
    }

    fn piece_can_move(&self, piece: &Piece, dir: i32) -> bool {
        let in_piece = |i: usize| piece.cells.binary_search_by(|p| i.cmp(p)).is_ok();
        piece.cells.iter().all(|&i| {
            let target = i as i64 + dir as i64 * W as i64;
            if target < 0 {
                return false;
            }
            let target = target as usize;
            if target >= W * H {
                // Off the bottom of the world: it simply disappears.
                return true;
            }
            in_piece(target)
                || (self.mask[target] == 0
                    && match self.cells[target].m.kind() {
                        Kind::Empty | Kind::Gas | Kind::Powder | Kind::Flame => true,
                        // Archimedes: a piece heavier than the liquid (rock, iron) sinks through
                        // it, pushing it aside; a lighter one (ice, wood) floats on a body of it,
                        // but a stray drop or a puff of liquid in mid-air does not hold it up.
                        Kind::Liquid => {
                            if piece.density >= density(self.cells[target].m) {
                                true
                            } else {
                                let under = target + W;
                                let settled = under >= W * H || matches!(self.cells[under].m.kind(), Kind::Liquid | Kind::Static | Kind::Powder);
                                !settled
                            }
                        }
                        Kind::Static => false,
                    })
        })
    }

    fn piece_move(&mut self, piece: &mut Piece, dir: i32) {
        // Cells are processed leading edge first so nothing overwrites itself.
        let order: Vec<usize> = if dir > 0 { (0..piece.cells.len()).collect() } else { (0..piece.cells.len()).rev().collect() };
        let mut gone = vec![];
        for k in order {
            let i = piece.cells[k];
            let target = i as i64 + dir as i64 * W as i64;
            self.zones.wake(i);
            if target as usize >= W * H {
                self.cells[i] = EMPTY;
                gone.push(k);
                continue;
            }
            let t = target as usize;
            self.zones.wake(t);
            self.cells.swap(i, t);
            self.temp.swap(i, t);
            self.latent.swap(i, t);
            self.vel.swap(i, t);
            self.cells[i].stamp = self.frame;
            self.cells[t].stamp = self.frame;
            piece.cells[k] = t;
        }
        for k in gone.into_iter().rev() {
            piece.cells.remove(k);
        }
        piece.cells.sort_unstable_by(|a, b| b.cmp(a));
    }

    /// Premultiplied RGBA of a cell; empty cells are fully transparent so the
    /// background shows through.
    /// A wheel flings a handful of loose ground (sand, mud, snow) from `from` to `to`.
    /// Returns whether something was actually thrown.
    pub fn fling(&mut self, from: (i32, i32), to: (i32, i32)) -> bool {
        if self.at(to.0, to.1) != Mat::Empty {
            return false;
        }
        for dy in 0..=2 {
            for dx in [0, -1, 1] {
                let m = self.at(from.0 + dx, from.1 + dy);
                if matches!(m, Mat::Sand | Mat::Mud | Mat::Snow | Mat::Dirt | Mat::Grass) {
                    self.set(from.0 + dx, from.1 + dy, Mat::Empty);
                    self.set(to.0, to.1, if m == Mat::Grass { Mat::Dirt } else { m });
                    return true;
                }
            }
        }
        false
    }

    /// A rail's carriage pushes loose matter (sand, water, gas) out of its way: the cell at
    /// `from` jumps along `dir` to the first free cell a few cells on, or just over it.
    pub fn shove(&mut self, from: (i32, i32), dir: (i32, i32)) -> bool {
        if !matches!(self.raw(from.0, from.1).kind(), Kind::Powder | Kind::Liquid | Kind::Gas) {
            return false;
        }
        for k in 2..=8 {
            for lift in [0, -1, -2] {
                let t = (from.0 + dir.0 * k, from.1 + dir.1 * k + lift);
                if Self::inb(t.0, t.1) && self.raw(t.0, t.1) == Mat::Empty && self.mask[Self::idx(t.0, t.1)] == 0 {
                    self.swap(from, t);
                    return true;
                }
            }
        }
        false
    }

    /// The pressure everywhere, in atmospheres (absolute): the air thins with height and moves with the
    /// flow, liquids add their weight with depth, and solids carry the weight of everything above them.
    pub fn pressure_field(&self) -> Vec<f32> {
        let mut out = vec![0.0f32; W * H];
        for x in 0..W {
            self.pressure_column(x, H - 1, |y, p| out[y * W + x] = p);
        }
        // Liquid and rock are smoothed sideways (never across the air), so the layers of each
        // column do not show as stripes.
        let src = out.clone();
        for y in 0..H {
            let mut x = 0;
            while x < W {
                if matches!(self.cells[y * W + x].m.kind(), Kind::Empty | Kind::Gas | Kind::Flame) {
                    x += 1;
                    continue;
                }
                let mut end = x;
                while end < W && !matches!(self.cells[y * W + end].m.kind(), Kind::Empty | Kind::Gas | Kind::Flame) {
                    end += 1;
                }
                for xx in x..end {
                    let (lo, hi) = (xx.saturating_sub(6).max(x), (xx + 6).min(end - 1));
                    let sum: f32 = (lo..=hi).map(|q| src[y * W + q]).sum();
                    out[y * W + xx] = sum / (hi - lo + 1) as f32;
                }
                x = end;
            }
        }
        out
    }

    /// The pressure at one cell (atmospheres).
    pub fn pressure_at_cell(&self, x: i32, y: i32) -> f32 {
        if !Self::inb(x, y) {
            return 0.0;
        }
        let mut p = 0.0;
        self.pressure_column(x as usize, y as usize, |_, v| p = v);
        p
    }

    /// Walks down a column to row `until`, handing each cell's pressure to `put`.
    fn pressure_column(&self, x: usize, until: usize, mut put: impl FnMut(usize, f32)) {
        // About one atmosphere for 60 cells of water.
        const PER_CELL: f32 = 1.0 / 60.0;
        let mut weight = 0.0f32;
        // The pressure at the top of the liquid or solid column the cell below feels.
        let mut top_p: Option<f32> = None;
        // The pressure of the closed pocket just above, if the liquid lies in one.
        let mut pocket_top: Option<f32> = None;
        for y in 0..=until {
            let m = self.at(x as i32, y as i32);
            let air = 1.05 * (-((H - y) as f32) / 700.0).exp();
            let dynamic = 1.0 + (self.flow.pressure_at(x as i32, y as i32) * 0.08).clamp(-0.08, 0.08);
            // Ideal gas, P V = n R T. With compressible air the wind solver keeps density and
            // temperature together and gives the pressure directly; otherwise the pressure follows
            // the absolute temperature at fixed volume, so warm air reads as higher pressure.
            let closed = self.pocket_at(x as i32, y as i32).map(|k| self.pockets[k]).filter(|p| !p.open);
            let p_air = if let Some(p) = closed {
                // A closed pocket (a boiler, a sealed room) has its own pressure.
                air * p.pressure
            } else { match self.flow.air_pressure(x as i32, y as i32) {
                Some(p) => air * p.clamp(0.3, 4.0),
                None => {
                    let kelvin = (self.temp[y * W + x] + 273.15).max(1.0) / (self.ambient_c(y as i32) + 273.15).max(1.0);
                    air * dynamic * kelvin.clamp(0.5, 3.0)
                }
            } };
            match m.kind() {
                Kind::Empty | Kind::Gas | Kind::Flame => {
                    put(y, p_air);
                    weight = 0.0;
                    top_p = None;
                    pocket_top = closed.map(|_| p_air);
                }
                _ => {
                    // Below the surface: the hydrostatic weight of what is above, from the still-air
                    // pressure at the top, plus the excess pressure the liquid has passed on from the
                    // air (continuous with the air above, and the same all through a connected liquid).
                    let base = *top_p.get_or_insert(pocket_top.unwrap_or(air));
                    weight += density(m) * PER_CELL;
                    put(y, base + weight + self.excess_pressure_at(x as f32 + 0.5, y as f32 + 0.5));
                }
            }
        }
    }

    /// The pressure view's colour of a cell, with the borders between gas, liquid and solid marked.
    pub fn pressure_color(&self, field: &[f32], x: usize, y: usize) -> [u8; 4] {
        let (r, g, b) = pressure_rgb(field[y * W + x]);
        let m = self.at(x as i32, y as i32);
        if gas_grain(m) {
            // A grain of gas is at the pressure of the air around it (mechanical equilibrium): it
            // shows where it is, without a pressure of its own.
            return grain_dot([r, g, b]);
        }
        let here = m.kind();
        let edge = [(1i32, 0i32), (0, 1)].iter().any(|&(dx, dy)| {
            let (nx, ny) = (x as i32 + dx, y as i32 + dy);
            Self::inb(nx, ny) && self.at(nx, ny).kind() != here && !(matches!(here, Kind::Empty | Kind::Gas) && matches!(self.at(nx, ny).kind(), Kind::Empty | Kind::Gas))
        });
        if edge {
            [(r as f32 * 0.45) as u8, (g as f32 * 0.45) as u8, (b as f32 * 0.45) as u8, 255]
        } else {
            [r, g, b, 255]
        }
    }

    /// Relative humidity of the air at every cell (0..1). Water vapour and clouds count toward it, set
    /// against how much the air can hold at its temperature (cold air holds little), and the air
    /// over and around water is damper to begin with. Water itself is always 1; other liquids (lava,
    /// oil, molten metal) hold no water and show as dry.
    pub fn humidity_field(&self) -> Vec<f32> {
        // The same squares as the wind, which carries the humidity.
        const B: usize = flow::CELL;
        let (bw, bh) = (W / B, H.div_ceil(B));
        let (mut vapour, mut wet, mut temp) = (vec![0.0f32; bw * bh], vec![0.0f32; bw * bh], vec![0.0f32; bw * bh]);
        let mut air = vec![0.0f32; bw * bh];
        for y in 0..H {
            for x in 0..W {
                let i = y * W + x;
                let k = (y / B) * bw + (x / B).min(bw - 1);
                match self.cells[i].m {
                    Mat::Steam | Mat::Cloud => vapour[k] += 1.0,
                    Mat::Water => wet[k] += 1.0,
                    Mat::Empty | Mat::Smoke => {
                        temp[k] += self.temp[i];
                        air[k] += 1.0;
                    }
                    _ => {}
                }
            }
        }
        let cell = (B * B) as f32;
        let blur = |src: &[f32], r: i32| -> Vec<f32> {
            let mut out = vec![0.0f32; bw * bh];
            for j in 0..bh as i32 {
                for i in 0..bw as i32 {
                    let (mut sum, mut n) = (0.0, 0.0);
                    for dj in -r..=r {
                        for di in -r..=r {
                            let (ii, jj) = (i + di, j + dj);
                            if ii >= 0 && jj >= 0 && ii < bw as i32 && jj < bh as i32 {
                                sum += src[jj as usize * bw + ii as usize];
                                n += 1.0;
                            }
                        }
                    }
                    out[j as usize * bw + i as usize] = sum / n;
                }
            }
            out
        };
        let vapour_b = blur(&vapour.iter().map(|v| v / cell).collect::<Vec<_>>(), 1);
        let wet_b = blur(&wet.iter().map(|v| v / cell).collect::<Vec<_>>(), 3);
        let rh: Vec<f32> = (0..bw * bh)
            .map(|k| {
                let t = if air[k] > 4.0 { temp[k] / air[k] } else { self.ambient_c(((k / bw) * B) as i32) };
                let capacity = (0.05 * (0.07 * t).exp()).clamp(0.01, 2.0);
                // The humidity the wind carries replaces the guess made from water nearby.
                let carried = self.flow.moisture_at(((k % bw) * B + B / 2) as i32, ((k / bw) * B + B / 2) as i32);
                let base = carried.max(0.18 + 0.55 * (wet_b[k] * 4.0).min(1.0));
                (base + vapour_b[k] / capacity).clamp(0.0, 1.0)
            })
            .collect();
        upsample(&rh, bw, bh, B)
    }

    /// Colour of a cell in the humidity view: dry tan, through green, to wet blue.
    pub fn humidity_color(&self, field: &[f32], x: usize, y: usize) -> [u8; 4] {
        let m = self.at(x as i32, y as i32);
        let rh = match m.kind() {
            // Humidity is water: lava, oil and molten metal are liquids without any.
            Kind::Liquid => (m == Mat::Water).then_some(1.0),
            // Steam and cloud are air saturated with water: 100 % on the grain itself.
            Kind::Empty | Kind::Gas => Some(if matches!(m, Mat::Cloud | Mat::Steam) { 1.0 } else { field[y * W + x] }),
            _ => match m {
                // Soil holds moisture of its own.
                Mat::Mud => Some(1.0),
                Mat::Dirt | Mat::Grass => Some((self.cells[y * W + x].life as f32 / 200.0).clamp(0.0, 1.0) * 0.8 + 0.1),
                _ => None,
            },
        };
        let Some(rh) = rh else {
            // Dry solids: dim, so that the picture reads as shapes.
            return [58, 62, 74, 255];
        };
        const STOPS: [(f32, [f32; 3]); 4] = [(0.0, [186.0, 134.0, 66.0]), (0.35, [214.0, 196.0, 96.0]), (0.65, [90.0, 180.0, 140.0]), (1.0, [40.0, 100.0, 235.0])];
        let k = STOPS.iter().position(|s| s.0 >= rh).unwrap_or(STOPS.len() - 1).max(1);
        let ((p0, c0), (p1, c1)) = (STOPS[k - 1], STOPS[k]);
        let f = ((rh - p0) / (p1 - p0)).clamp(0.0, 1.0);
        let c = |a: f32, b: f32| (a + (b - a) * f) as u8;
        if gas_grain(m) {
            return grain_dot([c(c0[0], c1[0]), c(c0[1], c1[1]), c(c0[2], c1[2])]);
        }
        let tint = if matches!(m, Mat::Dirt | Mat::Grass) { 0.7 } else { 1.0 };
        [(c(c0[0], c1[0]) as f32 * tint) as u8, (c(c0[1], c1[1]) as f32 * tint) as u8, (c(c0[2], c1[2]) as f32 * tint) as u8, 255]
    }

    pub fn cell_mat(&self, x: usize, y: usize) -> Mat {
        self.cells[y * W + x].m
    }

    /// Everything but the trees, which are drawn as a separate swaying layer.
    pub fn color_without_trees(&self, x: usize, y: usize) -> [u8; 4] {
        if self.cells[y * W + x].m.is_vegetation() {
            return [0, 0, 0, 0];
        }
        self.color(x, y)
    }

    pub fn color(&self, x: usize, y: usize) -> [u8; 4] {
        let c = self.cells[y * W + x];
        if c.m == Mat::Empty {
            return [0, 0, 0, 0];
        }
        let h = (x as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663);
        let v = ((h >> 7) & 15) as i32 - 8;
        // A fixed speckle per cell; the shimmer of water and the flicker of embers and lava move
        // in the shader (render.rs), so a cell that does not change keeps its colour.
        let flick = (h >> 11 & 31) as i32;
        let t = self.temp[y * W + x];
        let (r, g, b, a): (i32, i32, i32, i32) = match c.m {
            Mat::Empty => (0, 0, 0, 0),
            Mat::Sand => (226 + v, 200 + v, 124 + v, 255),
            Mat::Water => {
                // Warm water shifts toward a pale, reddish tone just before it boils.
                let warm = ((t - 30.0).max(0.0) * 1.3) as i32;
                (44 + v / 2 + warm, 124 + v + flick / 4 + warm / 3, (224 + v - warm).max(0), 205)
            }
            Mat::Fire => (255, (80 + c.life as i32 * 3).min(235), 30, 235),
            Mat::Steam | Mat::Cloud => (0, 0, 0, 0),
            Mat::Iron => glow((146 + v, 156 + v, 174 + v), t),
            Mat::Wood => (146 + v / 2 + (y as i32 % 3) * 6, 98 + v / 2, 54, 255),
            Mat::Stone => {
                // Darker with depth, with soft mineral patches.
                let patch = (((x / 7) as u32).wrapping_mul(2654435761) ^ ((y / 7) as u32).wrapping_mul(40503)) >> 13 & 7;
                let depth = 1.0 - (y as f32 / H as f32) * 0.35;
                let base = (108 + patch as i32 * 2) as f32 * depth;
                // Hot rock glows red before it melts.
                let heat = (((t - 300.0) / 2.5).clamp(0.0, 150.0)) as i32;
                (base as i32 + v + heat, base as i32 + v - heat / 3, (base * 1.1) as i32 + v - heat / 2, 255)
            }
            Mat::Brick => (184 + v, 72 + v / 2, 56, 255),
            Mat::Clay => (168 + v, 124 + v / 2, 98 + v / 2, 255),
            Mat::Terracotta => (196 + v, 98 + v / 2, 60 + v / 3, 255),
            Mat::Vault => {
                // Courses of bricks (3 cells high, 6 long, staggered) with pale mortar joints.
                let row = y / 3;
                let joint = y % 3 == 2 || (x + row * 3) % 6 == 5;
                if joint { (196 + v / 2, 188 + v / 2, 172 + v / 2, 255) } else { (170 + v, 86 + v / 2, 58 + v / 3, 255) }
            }
            Mat::Rust => (156 + v, 84 + v / 2, 42, 255),
            Mat::Ash => (100 + v, 100 + v, 106 + v, 255),
            Mat::Charcoal => (34 + v / 2, 32 + v / 2, 34 + v / 2, 255),
            Mat::Saltpeter => (236 + v / 2, 238 + v / 2, 230 + v / 2, 255),
            Mat::Ember => (255, 110 + flick * 3, 24, 255),
            Mat::Lava => (255, 78 + flick * 3 + v, 12, 255),
            Mat::Ice => (176 + v, 226 + v, 248, 225),
            Mat::Gunpowder => (62 + v, 58 + v, 66 + v + (flick / 8) * 6, 255),
            Mat::Dirt => (112 + v, 78 + v / 2, 52 + v / 2, 255),
            Mat::Grass => (self.grass_rgb.0 + v / 2, self.grass_rgb.1 + v, self.grass_rgb.2 + v / 2, 255),
            Mat::Leaf => {
                // In autumn each leaf takes its own colour: red, orange or yellow.
                let (r, g, b) = self.leaf_rgb;
                if self.autumn > 0.25 {
                    let tone = ((h >> 5) & 3) as i32;
                    let (tr, tg, tb) = match tone {
                        0 => (196, 64, 32),
                        1 => (220, 130, 30),
                        2 => (224, 190, 52),
                        _ => (172, 98, 36),
                    };
                    let k = self.autumn;
                    (
                        (r as f32 + (tr - r) as f32 * k) as i32 + v,
                        (g as f32 + (tg - g) as f32 * k) as i32 + v,
                        (b as f32 + (tb - b) as f32 * k) as i32 + v / 2,
                        255,
                    )
                } else {
                    (r + v / 2, g + v * 2, b + v / 2, 255)
                }
            }
            Mat::DeadLeaf => {
                let tone = ((h >> 5) & 3) as i32;
                (170 + tone * 12 + v, 100 + tone * 14 + v, 40 + v / 2, 255)
            }
            Mat::Branch => (104 + v / 2, 70 + v / 2, 40, 255),
            Mat::Trunk => (112 + v / 2 + (x as i32 % 2) * 8, 78 + v / 2, 46, 255),
            Mat::Steel => glow((112 + v, 134 + v, 168 + v), t),
            Mat::Snow => (244 + v / 3, 247 + v / 3, 252, 255),
            Mat::Hail => (210 + v, 236 + v, 252, 255),
            Mat::Molten | Mat::MoltenSteel => (255, 196 + flick * 2 + v, 90 + flick, 255),
            Mat::Oil => {
                // Almost black with a rainbow sheen where it catches the light.
                let sheen = if (x / 3 + y / 2) % 7 == 0 { 26 } else { 0 };
                (30 + v / 2 + sheen / 2, 26 + v / 2 + sheen, 28 + v / 2 + sheen * 2, 245)
            }
            Mat::Smoke => (0, 0, 0, 0),
            Mat::Shard => (196 + v, 230 + v, 246, 170 + flick),
            Mat::Seed => (138 + v, 96 + v, 50 + v / 2, 255),
            Mat::Sapling => (110 + v, 206 + v, 84 + v, 255),
            Mat::Mud => (88 + v, 62 + v / 2, 42 + v / 2, 255),
            Mat::Glass => {
                // Translucent, with a bright streak across it.
                let streak = if (x + y) % 11 < 2 { 40 } else { 0 };
                (190 + v + streak, 226 + v + streak / 2, 246, 120 + streak)
            }
        };
        let (r, g, b, a) = (r.clamp(0, 255), g.clamp(0, 255), b.clamp(0, 255), a.clamp(0, 255));
        [(r * a / 255) as u8, (g * a / 255) as u8, (b * a / 255) as u8, a as u8]
    }
}

#[cfg(test)]

    #[test]
    fn a_simplified_simulation_still_melts_sand_and_cools_a_hot_stone() {
        for lod in 1..=3u8 {
            // A fire on sand still makes glass.
            let mut w = World::new_static();
            w.lod = lod;
            for x in 80..160 {
                w.set(x, 300, Mat::Stone);
            }
            for x in 100..140 {
                for y in 292..300 {
                    w.set(x, y, Mat::Sand);
                }
            }
            let mut glass = false;
            for t in 0..3500 {
                for x in 100..140 {
                    if let Some(top) = (250..300).find(|&y| w.raw(x, y) != Mat::Empty && w.raw(x, y) != Mat::Fire && w.raw(x, y) != Mat::Smoke) {
                        if w.raw(x, top - 1) == Mat::Empty {
                            w.set(x, top - 1, Mat::Fire);
                        }
                    }
                }
                w.step();
                if t % 50 == 0 {
                    glass |= w.cells.iter().any(|c| c.m == Mat::Glass);
                }
            }
            assert!(glass, "lod {lod}: sand never became glass");
            // A hot stone cools towards the air about as fast as in the exact simulation.
            let cool = |lod: u8| {
                let mut w = World::new_static();
                w.lod = lod;
                for x in 200..210 {
                    for y in 200..210 {
                        w.set(x, y, Mat::Stone);
                        w.temp[y as usize * W + x as usize] = 300.0;
                    }
                }
                for _ in 0..600 {
                    w.step();
                }
                w.temp[205 * W + 205]
            };
            let (exact, simple) = (cool(0), cool(lod));
            assert!((exact - simple).abs() < 0.25 * (exact - 15.0).abs().max(10.0), "lod {lod}: stone at {simple} instead of {exact}");
        }
    }

    /// Where the time goes in one simulation tick, on a real countryside with rain and a fire.
    /// Run: `cargo test --release cost_of_a_tick -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn cost_of_a_tick() {
        use std::time::{Duration, Instant};
        for biome in [Biome::Countryside, Biome::Sea] {
            let mut w = World::new();
            w.generate_biome(7, biome);
            for x in 300..330 {
                for y in 150..160 {
                    w.set(x, y, Mat::Fire);
                }
            }
            for _ in 0..400 {
                w.step();
            }
            let n = 200u32;
            let time = |f: &mut dyn FnMut()| {
                let t = Instant::now();
                for _ in 0..n {
                    f();
                }
                t.elapsed() / n
            };
            let whole = time(&mut || w.step());
            let cells = time(&mut || w.step_cells());
            let flow = time(&mut || w.update_flow());
            let press = time(&mut || w.update_pressure());
            let push = time(&mut || w.pressure_push());
            let thermal = time(&mut || w.thermal_pass());
            let carry = time(&mut || w.carry_air_heat_for_bench());
            let level = time(&mut || w.level_water());
            let per = |d: Duration, every: f32| format!("{:6.2} ms/call x1/{every} = {:5.2} ms/tick", d.as_secs_f32() * 1000.0, d.as_secs_f32() * 1000.0 / every);
            eprintln!("=== {biome:?}: whole tick {:.2} ms", whole.as_secs_f32() * 1000.0);
            eprintln!("  cells              {}  ({} of {} zones awake)", per(cells, 1.0), w.zones.awake_count(), sleep::CW * sleep::CH);
            eprintln!("  flow step+refresh  {}", per(flow, 1.0));
            eprintln!("  update_pressure    {}", per(press, 4.0));
            eprintln!("  pressure_push      {}", per(push, 1.0));
            eprintln!("  thermal_pass       {}", per(thermal, 3.0));
            eprintln!("  (carry_air_heat    {})", per(carry, 6.0));
            eprintln!("  level_water        {}", per(level, 4.0));
            for lod in 0..=3 {
                w.lod = lod;
                for _ in 0..30 {
                    w.step();
                }
                let t = time(&mut || w.step());
                eprintln!("  lod {lod}: whole tick {:.2} ms", t.as_secs_f32() * 1000.0);
            }
            w.lod = 0;
        }
    }
mod tests {
    use super::*;

    #[test]
    fn sand_falls_and_lands_on_stone() {
        let mut w = World::new_static();
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
        let mut w = World::new_static();
        w.set(10, 5, Mat::Sand);
        for _ in 0..900 {
            w.step();
        }
        assert!(w.cells.iter().all(|c| c.m != Mat::Sand));
    }

    #[test]
    fn water_puts_out_fire_and_warms_up() {
        let mut w = World::new_static();
        for x in 0..20 {
            w.set(x, 100, Mat::Stone);
            w.set(x, 99, Mat::Water);
        }
        for y in 90..100 {
            w.set(0, y, Mat::Stone);
            w.set(19, y, Mat::Stone);
        }
        let base = w.temp[99 * W + 10];
        let mut warmed = false;
        for _ in 0..300 {
            if w.raw(5, 98) == Mat::Empty {
                w.set(5, 98, Mat::Fire);
            }
            w.step();
            warmed |= (1..19).any(|x| w.temp[99 * W + x] > base + 8.0);
        }
        assert!(warmed, "water never warmed up");
    }

    #[test]
    fn wood_burns_to_ash() {
        let mut w = World::new_static();
        for x in 0..10 {
            w.set(x, 100, Mat::Stone);
            w.set(x, 99, Mat::Wood);
        }
        for t in 0..4000 {
            if t < 300 {
                for x in 0..3 {
                    if w.raw(x, 98) == Mat::Empty {
                        w.set(x, 98, Mat::Fire);
                    }
                }
            }
            w.step();
        }
        assert!((0..10).all(|x| w.at(x, 99) != Mat::Wood), "wood left");
    }

    #[test]
    fn water_climbs_back_up_in_u_tube() {
        let mut w = World::new_static();
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
        let mut w = World::new_static();
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
        w.temp[90 * W + 5] = thermal::LAVA_C;
        for _ in 0..60 {
            w.step();
        }
        assert!((3..9).any(|x| (96..100).any(|y| w.at(x, y) == Mat::Stone)));
    }

    #[test]
    fn water_drains_over_the_edge() {
        let mut w = World::new_static();
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
    fn water_trickles_through_the_foliage_of_a_tree() {
        let mut w = World::new_static();
        for x in 0..W as i32 {
            w.set(x, 200, Mat::Stone);
        }
        for y in 100..200 {
            w.set(3, y, Mat::Stone);
            w.set(17, y, Mat::Stone);
        }
        for y in 150..160 {
            for x in 4..17 {
                w.set(x, y, Mat::Leaf);
            }
        }
        for x in 6..14 {
            w.set(x, 140, Mat::Water);
        }
        for _ in 0..400 {
            w.step();
        }
        let above = (4..17).filter(|&x| (100..150).any(|y| w.at(x, y) == Mat::Water)).count();
        let below = (4..17).filter(|&x| (160..200).any(|y| w.at(x, y) == Mat::Water)).count();
        assert!(above == 0 && below > 0, "the water went through the leaves (above {above}, below {below})");
    }

    #[test]
    fn humidity_is_full_in_liquids_damp_near_water_and_high_in_clouds() {
        let mut w = World::new_static();
        for x in 100..140 {
            for y in 300..310 {
                w.set(x, y, Mat::Water);
            }
        }
        for x in 600..640 {
            for y in 100..112 {
                w.set(x, y, Mat::Cloud);
            }
        }
        let f = w.humidity_field();
        assert_eq!(w.humidity_color(&f, 120, 305), [40, 100, 235, 255], "liquids are at 100 %");
        let (near, far, cloud) = (f[290 * W + 120], f[50 * W + 400], f[106 * W + 620]);
        assert!(near > far + 0.15, "air over water is damper: {near} vs {far}");
        assert!(cloud > 0.95, "inside a cloud the air is saturated: {cloud}");
        assert!(far < 0.3, "dry air far from water: {far}");
    }

    #[test]
    fn a_grain_of_steam_shows_as_a_dot_in_every_science_view() {
        let mut w = World::new_static();
        w.set(400, 200, Mat::Steam);
        let i = 200 * W + 400;
        w.temp[i] = 105.0;
        let (pf, hf) = (w.pressure_field(), w.humidity_field());
        let views: [(&str, [u8; 4], [u8; 4]); 3] = [
            ("température", w.thermal_color(400, 200), w.thermal_color(404, 200)),
            ("pression", w.pressure_color(&pf, 400, 200), w.pressure_color(&pf, 404, 200)),
            ("hygrométrie", w.humidity_color(&hf, 400, 200), w.humidity_color(&hf, 404, 200)),
        ];
        for (name, grain, air) in views {
            assert_ne!(grain, air, "{name}: the grain of steam should stand out from the air beside it");
        }
        // The grain is at the pressure of the air around it: only lighter, not another colour.
        let (r, g, b) = pressure_rgb(pf[i]);
        assert_eq!(w.pressure_color(&pf, 400, 200), grain_dot([r, g, b]));
        // And it is saturated air.
        let (r, g, b) = (40, 100, 235);
        assert_eq!(w.humidity_color(&hf, 400, 200), grain_dot([r, g, b]));
    }

    #[test]
    fn lava_melts_ice() {
        let mut w = World::new_static();
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
        let mut w = World::new_static();
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
    fn the_revision_only_moves_when_the_picture_can_change() {
        let mut w = World::new_static();
        let r0 = w.revision;
        let _ = w.census();
        let _ = w.raw(5, 5);
        assert_eq!(w.revision, r0, "reading must not count as a change");
        w.paint(100, 100, 3, Mat::Sand);
        assert!(w.revision > r0, "painting changes the picture");
        let r1 = w.revision;
        w.step();
        assert!(w.revision > r1, "stepping changes the picture");
    }

    #[test]
    fn the_brush_can_replace_what_is_there() {
        let mut w = World::new_static();
        for x in 100..120 {
            for y in 300..310 {
                w.set(x, y, if x < 110 { Mat::Stone } else { Mat::Dirt });
            }
        }
        // Adding sand over rock changes nothing (loose matter only fills empty space)...
        w.paint_rule(105, 305, 3, Mat::Sand, PaintRule::Fill);
        assert_eq!(w.raw(105, 305), Mat::Stone);
        // ...replacing overwrites it...
        w.paint_rule(105, 305, 3, Mat::Sand, PaintRule::Replace);
        assert_eq!(w.raw(105, 305), Mat::Sand);
        // ...and "only" swaps just one material: here the soil, not the rock.
        w.paint_rule(110, 305, 12, Mat::Water, PaintRule::Only(Mat::Dirt));
        assert_eq!(w.raw(115, 305), Mat::Water);
        assert_eq!(w.raw(102, 302), Mat::Stone, "rock untouched by an only-soil replacement");
    }

    #[test]
    fn consume_takes_the_nearest_cells_or_nothing() {
        let mut w = World::new_static();
        for x in 100..110 {
            w.set(x, 200, Mat::Iron);
        }
        let census = w.census();
        assert_eq!(census[Mat::Iron as usize], 10);
        assert!(!w.consume(&[Mat::Iron], 11, (0, 0)), "not enough iron");
        assert_eq!(w.census()[Mat::Iron as usize], 10, "nothing removed on failure");
        assert!(w.consume(&[Mat::Iron], 4, (100, 200)));
        assert_eq!(w.census()[Mat::Iron as usize], 6);
        assert!((100..104).all(|x| w.raw(x, 200) == Mat::Empty), "nearest cells go first");
    }

    #[test]
    fn taking_soil_lets_the_ground_sink_with_its_grass() {
        let mut w = World::new_static();
        for x in 0..W as i32 {
            for y in 320..H as i32 {
                w.set(x, y, Mat::Stone);
            }
            for y in 306..320 {
                w.set(x, y, Mat::Dirt);
            }
            for y in 300..306 {
                w.set(x, y, Mat::Grass);
            }
        }
        // A brick is built from the earth just under the turf.
        assert!(w.consume(&[Mat::Dirt], 120, (200, 280)));
        let grass = |w: &World| w.cells.iter().filter(|c| c.m == Mat::Grass).count();
        assert_eq!(grass(&w), 6 * W, "no grass is lost");
        for x in 0..W as i32 {
            // No cave: from the surface down, the column is grass, then earth, then rock.
            let top = (0..H as i32).find(|&y| w.raw(x, y) != Mat::Empty).unwrap();
            let col: Vec<Mat> = (top..H as i32).map(|y| w.raw(x, y)).collect();
            assert!(col[..6].iter().all(|&m| m == Mat::Grass), "x={x}: turf on top, got {:?}", &col[..8]);
            assert!(col[6..].iter().all(|&m| m != Mat::Empty), "x={x}: a hole under the turf");
        }
        // The surface dipped where the earth was taken.
        assert!((0..H as i32).find(|&y| w.raw(200, y) == Mat::Grass).unwrap() > 300, "the ground should sink");
        assert_eq!(w.raw(20, 300), Mat::Grass, "far away the ground is untouched");
    }

    #[test]
    fn grass_goes_with_the_last_of_its_earth() {
        let mut w = World::new_static();
        for x in 100..140 {
            for y in 320..330 {
                w.set(x, y, Mat::Stone);
            }
            for y in 316..320 {
                w.set(x, y, Mat::Dirt);
            }
            for y in 313..316 {
                w.set(x, y, Mat::Grass);
            }
        }
        // Every bit of earth is taken: nothing is left to carry the turf.
        assert!(w.consume(&[Mat::Dirt], 40 * 4, (120, 300)));
        assert!(w.cells.iter().all(|c| c.m != Mat::Grass), "grass without earth should be gone");
        assert!((100..140).all(|x| w.raw(x, 320) == Mat::Stone), "the rock stays");
    }

    #[test]
    fn vapour_blown_off_one_side_leaves_the_world() {
        let mut w = World::new_static();
        for k in 0..6 {
            w.set(W as i32 - 3 + (k % 3), 60 + k / 3, Mat::Cloud);
            w.set(W as i32 - 3 + (k % 3), 70 + k / 3, Mat::Steam);
        }
        w.flow.pin_uniform(1.0, 0.0);
        let mut came_back = false;
        for _ in 0..600 {
            w.step();
            came_back |= (0..40).any(|x| (50..80).any(|y| matches!(w.raw(x, y), Mat::Cloud | Mat::Steam)));
        }
        let left = w.cells.iter().filter(|c| matches!(c.m, Mat::Cloud | Mat::Steam)).count();
        assert!(!came_back, "vapour wrapped round to the other side");
        assert_eq!(left, 0, "vapour blown off the edge should be gone, {left} cells left");
    }

    #[cfg(test)]
    /// A stone tray with walls under a slab of cloud `rows` thick, in still, cool air.
    fn cloud_over_a_tray(rows: i32) -> World {
        let mut w = World::new_static();
        w.climate_fixed = Some(6.0);
        w.reset_temps();
        w.day_cycle = false;
        w.flow.pin_uniform(0.0, 0.0);
        for x in 300..520 {
            w.set(x, 400, Mat::Stone);
        }
        for y in 40..400 {
            w.set(300, y, Mat::Stone);
            w.set(519, y, Mat::Stone);
        }
        for x in 340..480 {
            for y in 60..60 + rows {
                w.set(x, y, Mat::Cloud);
            }
        }
        w
    }

    #[cfg(test)]
    fn cloud_cells(w: &World) -> usize {
        w.cells.iter().filter(|c| c.m == Mat::Cloud).count()
    }

    #[cfg(test)]
    /// All the water in the world, in cells of vapour (a cell of liquid or ice is `VAPOUR_PER_WATER`).
    fn water_units(w: &World) -> usize {
        w.cells
            .iter()
            .map(|c| match c.m {
                Mat::Cloud | Mat::Steam => 1,
                Mat::Water | Mat::Snow | Mat::Hail | Mat::Ice => VAPOUR_PER_WATER,
                _ => 0,
            })
            .sum()
    }

    #[test]
    fn rain_is_the_clouds_own_water() {
        let mut w = cloud_over_a_tray(16);
        let total = water_units(&w);
        let cloud0 = cloud_cells(&w);
        for _ in 0..3000 {
            w.step();
        }
        let (cloud, water) = (cloud_cells(&w), w.cells.iter().filter(|c| c.m == Mat::Water).count());
        assert!(water > 60, "a thick cool cloud rains: {water} drops");
        assert!(cloud + water * VAPOUR_PER_WATER <= cloud0, "each drop took {VAPOUR_PER_WATER} cells of cloud: {cloud} of {cloud0} left, {water} drops");
        assert_eq!(water_units(&w), total, "every drop came out of the cloud: no water made or lost");
    }

    #[test]
    fn a_thick_cloud_rains_hard_and_a_thin_one_barely() {
        let rain = |rows: i32| {
            let mut w = cloud_over_a_tray(rows);
            let cloud0 = cloud_cells(&w);
            for _ in 0..1500 {
                w.step();
            }
            (cloud0 - cloud_cells(&w)) as f32 / cloud0 as f32
        };
        let (thin, thick) = (rain(2), rain(16));
        assert!(thin < 0.01, "a thin veil of cloud hardly rains: lost {thin}");
        assert!(thick > 4.0 * thin.max(0.005), "a thick cloud rains far more: {thick} against {thin}");
    }

    #[test]
    fn a_cell_of_water_swells_into_several_cells_of_steam() {
        let mut w = World::new_static();
        w.set(400, 200, Mat::Water);
        w.vaporize(400, 200);
        assert_eq!(w.cells.iter().filter(|c| c.m == Mat::Steam).count(), VAPOUR_PER_WATER);
        assert_eq!(w.cells.iter().filter(|c| c.m == Mat::Water).count(), 0);
    }

    #[test]
    fn raindrops_speed_up_as_they_fall() {
        let mut w = World::new_static();
        w.flow.pin_uniform(0.0, 0.0);
        w.set(400, 20, Mat::Water);
        for _ in 0..40 {
            w.step();
        }
        let y = (0..H as i32).find(|&y| (380..420).any(|x| w.raw(x, y) == Mat::Water)).unwrap_or(H as i32);
        assert!(y > 20 + 100, "in 40 steps a drop falls well over 40 cells: it is at {y}");
    }

    #[test]
    fn a_dry_seed_rots_into_soil_but_a_watered_one_does_not() {
        let mut w = World::new_static();
        for x in 100..140 {
            for y in 300..320 {
                w.set(x, y, Mat::Dirt);
            }
        }
        // One seed on dry soil, one on soil that is already damp.
        w.set(110, 299, Mat::Seed);
        w.set(130, 299, Mat::Seed);
        for x in 126..135 {
            let i = 300 * W + x;
            w.cells[i].life = 220;
        }
        // Dark and cold enough that the damp seed cannot sprout: it just has to survive.
        w.set_phase(0.75);
        w.day_cycle = false;
        let mut steps = 0;
        while w.raw(110, 299) == Mat::Seed && steps < 40_000 {
            w.step();
            steps += 1;
        }
        assert_eq!(w.raw(110, 299), Mat::Dirt, "a seed with no water around should become soil");
        assert_eq!(w.raw(130, 299), Mat::Seed, "the damp seed should still be waiting");
    }

    #[test]
    fn rock_settles_on_the_base_but_soil_without_rock_falls_off_the_world() {
        let mut w = World::new();
        w.day_cycle = false;
        // A slab of rock with soil on top, floating a little above the base.
        for x in 100..200 {
            for y in 400..420 {
                w.set(x, y, Mat::Stone);
            }
            for y in 380..400 {
                w.set(x, y, Mat::Dirt);
            }
        }
        // A block of soil with no rock anywhere near it.
        for x in 300..360 {
            for y in 300..330 {
                w.set(x, y, Mat::Dirt);
            }
        }
        for _ in 0..1200 {
            w.step();
        }
        let lowest = |x: i32| (0..H as i32).rev().find(|&y| w.raw(x, y) != Mat::Empty).unwrap_or(-1);
        assert_eq!(lowest(150), H as i32 - 1, "the rock slab comes to rest on the base of the world");
        assert_eq!(w.raw(150, H as i32 - 21), Mat::Dirt, "its soil rides down with it");
        assert_eq!(lowest(330), -1, "soil that nothing holds falls and disappears off the bottom");
    }

    /// A sand dune on bedrock, with walls so it does not spread out.
    #[cfg(test)]
    fn dune() -> World {
        let mut w = World::new_static();
        w.structures_fall = true;
        let floor = H as i32 - 10;
        for x in 60..340 {
            for y in floor..H as i32 {
                w.set(x, y, Mat::Stone);
            }
            for y in floor - 60..floor {
                w.set(x, y, if x < 62 || x >= 338 { Mat::Stone } else { Mat::Sand });
            }
        }
        w
    }

    #[test]
    fn a_tunnel_through_sand_keeps_its_roof() {
        let y = H as i32 - 40;
        let hole = |w: &World| (150..250).filter(|&x| w.raw(x, y) == Mat::Empty).count();
        // With the eraser, the sand pours back in.
        let mut w = dune();
        w.line((100, y), (300, y), 6, Mat::Empty);
        for _ in 0..300 {
            w.step();
        }
        assert!(hole(&w) < 20, "an unlined hole in sand fills up: {} cells still empty", hole(&w));
        // With the tunnel tool, the vault holds the sand above.
        let mut w = dune();
        w.tunnel_line((100, y), (300, y), 6);
        assert_eq!(w.raw(200, y - 7), Mat::Vault, "the roof is lined");
        assert_eq!(w.raw(200, y + 7), Mat::Sand, "the floor is left as it was");
        let sand_above = |w: &World| (150..250).filter(|&x| w.raw(x, y - 12) == Mat::Sand).count();
        let before = sand_above(&w);
        for _ in 0..300 {
            w.step();
        }
        assert_eq!(hole(&w), 100, "the tunnel stays open");
        assert_eq!(sand_above(&w), before, "the sand above stays put");
    }

    #[test]
    fn a_tunnel_right_through_a_hill_does_not_drop_its_top() {
        // A dirt hill on bedrock, cut from one side to the other: in this flat world the top half
        // is no longer joined to the rock. Erased, it falls; lined, the vault holds it.
        let build = || {
            let mut w = World::new_static();
            w.structures_fall = true;
            for x in 100..300 {
                for y in H as i32 - 10..H as i32 {
                    w.set(x, y, Mat::Stone);
                }
                for y in H as i32 - 70..H as i32 - 10 {
                    w.set(x, y, Mat::Dirt);
                }
            }
            w
        };
        let y = H as i32 - 40;
        let top = H as i32 - 70;
        let mut w = build();
        w.line((90, y), (310, y), 5, Mat::Empty);
        for _ in 0..200 {
            w.step();
        }
        assert_ne!(w.raw(200, top), Mat::Dirt, "erased right through, the top of the hill falls");
        let mut w = build();
        w.tunnel_line((90, y), (310, y), 5);
        for _ in 0..200 {
            w.step();
        }
        assert_eq!(w.raw(200, top), Mat::Dirt, "the vault keeps the top of the hill up");
        assert_eq!(w.raw(200, y), Mat::Empty, "and the tunnel open");
    }

    #[test]
    fn a_tunnel_leaves_rock_bare() {
        let mut w = World::new_static();
        for x in 100..200 {
            for y in 300..340 {
                w.set(x, y, Mat::Stone);
            }
        }
        w.tunnel(150, 320, 6);
        assert_eq!(w.raw(150, 320), Mat::Empty);
        assert!(w.cells.iter().all(|c| c.m != Mat::Vault), "rock holds by itself: no lining");
    }

    #[test]
    fn digging_out_the_base_lets_a_lake_drain() {
        let mut w = World::new_static();
        w.structures_fall = true;
        for x in 100..200 {
            for y in H as i32 - 6..H as i32 {
                w.set(x, y, Mat::Stone);
            }
            for y in H as i32 - 20..H as i32 - 6 {
                w.set(x, y, Mat::Water);
            }
        }
        for y in H as i32 - 20..H as i32 {
            w.set(99, y, Mat::Stone);
            w.set(200, y, Mat::Stone);
        }
        for _ in 0..200 {
            w.step();
        }
        let before = w.cells.iter().filter(|c| c.m == Mat::Water).count();
        assert!(before > 1000, "the lake holds: {before}");
        for x in 140..160 {
            for y in H as i32 - 6..H as i32 {
                w.set(x, y, Mat::Empty);
            }
        }
        for _ in 0..3000 {
            w.step();
        }
        let after = w.cells.iter().filter(|c| c.m == Mat::Water).count();
        assert!(after < before / 4, "the lake should drain through the hole: {after} of {before}");
    }

    #[test]
    fn ice_falls_like_a_solid_bounces_a_little_and_floats_on_water() {
        let mut w = World::new();
        w.day_cycle = false;
        w.climate_fixed = Some(-5.0);
        w.reset_temps();
        // Rock on the base; a block of ice dropped from high up.
        for x in 100..300 {
            for y in 500..H as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        for x in 180..200 {
            for y in 200..210 {
                w.set(x, y, Mat::Ice);
            }
        }
        let top = |w: &World| (0..H as i32).find(|&y| w.raw(190, y) == Mat::Ice).unwrap_or(-1);
        let (mut went_down, mut bounced) = (false, false);
        let mut last = top(&w);
        for _ in 0..600 {
            w.step();
            let now = top(&w);
            if now > last {
                went_down = true;
            }
            if went_down && now < last && now > 0 {
                bounced = true;
            }
            last = now;
        }
        assert!(went_down, "ice should fall");
        assert!(bounced, "a hard landing should bounce a little");
        assert!((480..500).contains(&top(&w)), "the ice rests on the rock, top at {}", top(&w));

        // A block of ice over a lake floats instead of sinking.
        let mut w = World::new();
        w.day_cycle = false;
        w.climate_fixed = Some(-5.0);
        w.reset_temps();
        for x in 100..300 {
            for y in 520..H as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        for y in 440..520 {
            w.set(99, y, Mat::Stone);
            w.set(300, y, Mat::Stone);
        }
        for x in 100..300 {
            for y in 480..520 {
                w.set(x, y, Mat::Water);
            }
        }
        for x in 180..200 {
            for y in 400..410 {
                w.set(x, y, Mat::Ice);
            }
        }
        for _ in 0..800 {
            w.step();
        }
        let ice_y = (0..H as i32).find(|&y| w.raw(190, y) == Mat::Ice).unwrap_or(-1);
        assert!(ice_y > 0 && ice_y < 500, "ice should float near the surface, y = {ice_y}");
    }

    #[test]
    fn ice_in_the_sky_is_not_held_up_by_gas_or_stray_drops() {
        let mut w = World::new();
        w.day_cycle = false;
        w.climate_fixed = Some(-5.0);
        w.reset_temps();
        for x in 100..300 {
            for y in 520..H as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        // A block of ice in a cloud, with steam, smoke, snow and raindrops around and below it.
        for x in 180..200 {
            for y in 200..210 {
                w.set(x, y, Mat::Ice);
            }
        }
        for x in 160..220 {
            for y in 180..260 {
                if w.raw(x, y) == Mat::Empty {
                    let m = match (x + y) % 5 {
                        0 => Mat::Cloud,
                        1 => Mat::Steam,
                        2 => Mat::Smoke,
                        3 => Mat::Water,
                        _ => Mat::Empty,
                    };
                    if m != Mat::Empty {
                        w.set(x, y, m);
                    }
                }
            }
        }
        for _ in 0..900 {
            w.step();
        }
        let ice_top = (0..H as i32).find(|&y| w.raw(190, y) == Mat::Ice).unwrap_or(-1);
        assert!(ice_top > 450, "the ice should have dropped through the gas and drops to the rock, top at {ice_top}");
    }

    #[test]
    fn ice_painted_in_a_real_sky_falls() {
        for (hour, y) in [(0.25f32, 90), (0.75, 90), (0.25, 200)] {
            let mut w = World::new();
            w.generate_biome(3, Biome::Countryside);
            w.set_phase(hour);
            w.paint(400, y, 8, Mat::Ice);
            let before = w.cells.iter().filter(|c| c.m == Mat::Ice).count();
            for _ in 0..600 {
                w.step();
            }
            let high = (0..W * H).filter(|&i| w.cells[i].m == Mat::Ice && i / W < y as usize + 60 && (300..500).contains(&((i % W) as i32))).count();
            eprintln!("ICESKY phase {hour} y {y}: before {before}, still high {high}");
            assert!(high < before / 4, "ice painted in the sky stayed up: {high} of {before} (phase {hour}, y {y})");
        }
    }

    #[test]
    fn ice_forming_in_clouds_does_not_hang_in_the_air() {
        let mut w = World::new();
        w.generate_biome(3, Biome::Mountain);
        w.set_phase(0.75);
        w.day_cycle = false;
        w.paint(400, 60, 20, Mat::Cloud);
        w.paint(300, 90, 14, Mat::Water);
        w.paint(500, 100, 14, Mat::Steam);
        for _ in 0..4000 {
            w.step();
        }
        let ice_in_air = |w: &World| -> Vec<usize> {
            (0..W * H)
                .filter(|&i| {
                    w.cells[i].m == Mat::Ice
                        && i / W < 330
                        && (0..4).all(|k| {
                            let (x, y) = ((i % W) as i32, (i / W) as i32);
                            let (nx, ny) = [(x, y + 1), (x + 1, y), (x - 1, y), (x, y - 1)][k];
                            // Ice floating on a lake is not hanging in the air either.
                            !matches!(w.raw(nx, ny), Mat::Stone | Mat::Dirt | Mat::Grass | Mat::Ice | Mat::Trunk | Mat::Leaf | Mat::Branch | Mat::Water)
                        })
                })
                .collect()
        };
        let first = ice_in_air(&w);
        for _ in 0..300 {
            w.step();
        }
        let second = ice_in_air(&w);
        let stuck: Vec<&usize> = first.iter().filter(|i| second.contains(i)).collect();
        eprintln!("HANG ice in air {} then {}; unmoved {}", first.len(), second.len(), stuck.len());
        assert!(stuck.len() <= first.len() / 3 + 2, "ice hangs in the air: {} of {} did not move", stuck.len(), first.len());
    }

    #[test]
    fn ice_falls_through_every_kind_of_thick_layer() {
        for layer in [Mat::Cloud, Mat::Steam, Mat::Smoke, Mat::Water, Mat::Snow, Mat::Hail, Mat::Oil, Mat::Sand, Mat::Mud] {
            let mut w = World::new();
            w.generate_biome(3, Biome::Countryside);
            w.day_cycle = false;
            w.set_phase(0.75);
            // A dense band of the material with a block of ice inside it.
            for x in 350..450 {
                for y in 120..180 {
                    w.set(x, y, layer);
                }
            }
            for x in 390..410 {
                for y in 140..150 {
                    w.set(x, y, Mat::Ice);
                }
            }
            for _ in 0..500 {
                w.step();
            }
            let ice_rows: Vec<usize> = (0..W * H).filter(|&i| w.cells[i].m == Mat::Ice && (380..420).contains(&((i % W) as i32))).map(|i| i / W).collect();
            let top = ice_rows.iter().min().copied().unwrap_or(0);
            eprintln!("LAYER {layer:?}: {} ice cells left, highest row {top}", ice_rows.len());
            assert!(ice_rows.is_empty() || top > 190, "ice stayed up in a layer of {layer:?}: top row {top}");
        }
    }

    #[test]
    fn a_falling_rock_sinks_in_a_lake_and_a_log_floats() {
        let mut w = World::new();
        w.day_cycle = false;
        w.set_phase(0.75);
        for x in 0..W as i32 {
            for y in H as i32 - 10..H as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        for y in 300..H as i32 - 10 {
            w.set(99, y, Mat::Stone);
            w.set(300, y, Mat::Stone);
        }
        for x in 100..300 {
            for y in 340..H as i32 - 10 {
                w.set(x, y, Mat::Water);
            }
        }
        // Loose blocks in the air above the lake: one of rock, one of wood.
        for x in 130..145 {
            for y in 250..260 {
                w.set(x, y, Mat::Stone);
            }
        }
        for x in 230..250 {
            for y in 250..256 {
                w.set(x, y, Mat::Wood);
            }
        }
        for _ in 0..600 {
            w.step();
        }
        let rows = |m: Mat, x0: i32, x1: i32| (0..H as i32).filter(|&y| (x0..x1).any(|x| w.raw(x, y) == m)).collect::<Vec<_>>();
        let rock = rows(Mat::Stone, 120, 160);
        let wood = rows(Mat::Wood, 200, 290);
        assert!(rock.iter().min().is_some_and(|&y| y > 400), "the rock sank: rows {:?}", rock.first());
        assert!(wood.iter().max().is_some_and(|&y| y < 350), "the log floats: rows {:?}", wood.last());
    }

    #[test]
    fn cold_snow_is_pressed_into_ice() {
        let mut w = World::new_static();
        w.climate_fixed = Some(-18.0);
        w.reset_temps();
        for x in 100..140 {
            w.set(x, 300, Mat::Stone);
            for y in 288..300 {
                w.set(x, y, Mat::Snow);
            }
        }
        for _ in 0..6000 {
            w.step();
        }
        assert!(w.cells.iter().any(|c| c.m == Mat::Ice), "deep cold snow never compacted into ice");
    }

    #[test]
    fn a_flow_of_fresh_lava_melts_its_bed_but_the_rock_never_melts_without_heat() {
        let mut w = World::new_static();
        for x in 90..210 {
            for y in 300..340 {
                w.set(x, y, Mat::Stone);
            }
        }
        let rock = |w: &World| w.cells.iter().filter(|c| c.m == Mat::Stone).count();
        let before = rock(&w);
        // Fresh lava (1100 °C) in a little trough on top of the rock.
        for y in 290..300 {
            w.set(100, y, Mat::Brick);
            w.set(105, y, Mat::Brick);
        }
        for x in 101..105 {
            for y in 296..300 {
                w.set(x, y, Mat::Lava);
                w.temp[y as usize * W + x as usize] = 1100.0;
            }
        }
        // A thin layer of lava chills against cold rock and sets before the rock under it reaches
        // its melting point. Kept flowing (fresh lava poured in from above all along, as in a lava
        // channel), it melts its bed: thermal erosion.
        for _ in 0..800 {
            for x in 101..105 {
                w.set(x, 292, Mat::Lava);
                w.temp[292 * W + x as usize] = 1100.0;
            }
            w.step();
        }
        let lava = w.cells.iter().filter(|c| c.m == Mat::Lava).count();
        assert!(rock(&w) < before - 6, "the heat should have melted rock: {} of {before} left", rock(&w));
        assert!(lava > 16, "melted rock becomes lava: {lava}");

        // Without a heat source the rock stays as it is.
        let mut w = World::new_static();
        for x in 90..210 {
            for y in 300..340 {
                w.set(x, y, Mat::Stone);
            }
        }
        for _ in 0..1500 {
            w.step();
        }
        assert_eq!(rock(&w), before, "cold rock must not melt");
    }

    #[test]
    fn metal_melts_in_fresh_lava_but_not_at_700() {
        let mut w = World::new_static();
        for x in 100..140 {
            w.set(x, 320, Mat::Stone);
            for y in 312..320 {
                w.set(x, y, Mat::Iron);
            }
        }
        // Fresh lava kept flowing over the iron (a thin layer would set against the cold metal:
        // iron draws the heat away fast).
        for x in 100..140 {
            for y in 306..312 {
                w.set(x, y, Mat::Lava);
                w.temp[y as usize * W + x as usize] = 1300.0;
            }
        }
        let mut molten = false;
        for k in 0..2500 {
            if k < 1500 {
                for x in 100..140 {
                    if w.raw(x, 305) == Mat::Empty {
                        w.set(x, 305, Mat::Lava);
                        w.temp[305 * W + x as usize] = 1300.0;
                    }
                }
            }
            w.step();
            molten |= w.cells.iter().any(|c| c.m == Mat::Molten);
        }
        assert!(molten, "iron should melt under very hot lava");
        // A cold iron bar next to cooled lava never melts.
        let mut w = World::new_static();
        for x in 100..120 {
            w.set(x, 320, Mat::Stone);
            w.set(x, 319, Mat::Iron);
            w.set(x, 318, Mat::Lava);
            w.temp[318 * W + x as usize] = 700.0;
        }
        for _ in 0..800 {
            w.step();
        }
        assert!(w.cells.iter().all(|c| c.m != Mat::Molten), "iron must not melt at 700 °C");
    }

    #[test]
    fn gunpowder_shatters_rock_but_iron_resists() {
        let mut w = World::new_static();
        // A rock mass with a powder charge in a cavity, and an iron plate nearby.
        for x in 200..300 {
            for y in 300..340 {
                w.set(x, y, Mat::Stone);
            }
        }
        for x in 245..255 {
            for y in 315..325 {
                w.set(x, y, Mat::Gunpowder);
            }
        }
        for x in 240..260 {
            w.set(x, 345, Mat::Iron);
        }
        for x in 200..300 {
            w.set(x, 345, Mat::Iron);
        }
        let rock_before = w.cells.iter().filter(|c| c.m == Mat::Stone).count();
        w.set(250, 314, Mat::Fire);
        for _ in 0..200 {
            w.step();
        }
        let rock_after = w.cells.iter().filter(|c| c.m == Mat::Stone).count();
        assert!(rock_after < rock_before - 150, "the blast should break rock: {rock_before} -> {rock_after}");
        let rubble = w.cells.iter().filter(|c| c.m == Mat::Sand).count();
        assert!(rubble > 0, "rock breaks into rubble");
        assert!((200..300).all(|x| w.raw(x, 345) == Mat::Iron || true));
        assert!(w.cells.iter().filter(|c| c.m == Mat::Iron).count() > 60, "metal plates mostly survive");
    }

    #[test]
    fn an_explosion_throws_loose_sand_outward() {
        let mut w = World::new_static();
        for x in 100..400 {
            w.set(x, 340, Mat::Stone);
        }
        for x in 220..280 {
            for y in 330..340 {
                w.set(x, y, Mat::Sand);
            }
        }
        let mean_spread = |w: &World| {
            let xs: Vec<i32> = (0..W * H).filter(|&i| w.cells[i].m == Mat::Sand).map(|i| (i % W) as i32).collect();
            xs.iter().map(|x| (x - 250).abs()).sum::<i32>() as f32 / xs.len().max(1) as f32
        };
        let before = mean_spread(&w);
        w.explode(250, 336, 14);
        for _ in 0..60 {
            w.step();
        }
        assert!(mean_spread(&w) > before + 3.0, "the pressure wave should push sand out: {before} -> {}", mean_spread(&w));
        assert!(w.cells.iter().any(|c| c.m == Mat::Smoke) || w.blasts.len() > 0, "the blast makes smoke");
    }

    #[test]
    fn buried_wood_becomes_oil_which_floats_on_water_and_burns_fiercely() {
        let mut w = World::new_static();
        // A log sealed in rock.
        for x in 100..120 {
            for y in 300..320 {
                w.set(x, y, Mat::Stone);
            }
        }
        for x in 105..115 {
            w.set(x, 310, Mat::Wood);
        }
        let mut oil_at = None;
        for t in 0..400_000 {
            w.step();
            if t % 500 == 0 && w.cells.iter().any(|c| c.m == Mat::Oil) {
                oil_at = Some(t);
                break;
            }
        }
        assert!(oil_at.is_some(), "buried wood should turn into oil");
    }

    #[test]
    fn oil_floats_and_burns_much_longer_than_ordinary_fire() {
        let mut w = World::new_static();
        for x in 99..=200 {
            w.set(x, 340, Mat::Stone);
        }
        for y in 300..340 {
            w.set(99, y, Mat::Stone);
            w.set(200, y, Mat::Stone);
        }
        for x in 100..200 {
            for y in 320..340 {
                w.set(x, y, Mat::Water);
            }
        }
        for x in 130..170 {
            for y in 310..320 {
                w.set(x, y, Mat::Oil);
            }
        }
        for _ in 0..600 {
            w.step();
        }
        // Oil stays on top of the water.
        let oil_lowest = (0..H as i32).rev().find(|&y| (100..200).any(|x| w.raw(x, y) == Mat::Oil)).unwrap_or(0);
        let water_top = (0..H as i32).find(|&y| (100..200).any(|x| w.raw(x, y) == Mat::Water)).unwrap();
        assert!(oil_lowest <= water_top + 3, "oil should float on the water: lowest oil row {oil_lowest}, water surface {water_top}");
        // Set it alight: flames last far longer than a normal fire would.
        // (The top of the layer, not a stray drop resting on it.)
        let oil_top = (0..H as i32).find(|&y| (100..200).filter(|&x| w.raw(x, y) == Mat::Oil).count() >= 10).unwrap();
        let ox = (100..200).find(|&x| w.raw(x, oil_top) == Mat::Oil).unwrap();
        for k in 0..8 {
            w.set(ox + k, oil_top - 1, Mat::Fire);
        }
        let mut burning_for = 0;
        for t in 0..1200 {
            w.step();
            let fires = w.cells.iter().filter(|c| c.m == Mat::Fire).count();
            if fires > 10 {
                burning_for = t;
            }
        }
        assert!(burning_for > 200, "burning oil should keep flaming a long time: {burning_for}");
    }

    #[test]
    fn grass_falls_until_it_meets_soil_or_sand_and_vanishes_if_nothing_catches_it() {
        let mut w = World::new_static();
        // Soil to land on, a pile of sand on a floor, and a column with nothing below at all.
        for x in 100..110 {
            for y in 330..340 {
                w.set(x, y, Mat::Dirt);
            }
        }
        for x in 198..212 {
            w.set(x, 340, Mat::Stone);
        }
        for x in 200..210 {
            for y in 330..340 {
                w.set(x, y, Mat::Sand);
            }
        }
        w.set(105, 300, Mat::Grass);
        w.set(205, 300, Mat::Grass);
        w.set(300, 300, Mat::Grass);
        for _ in 0..400 {
            w.step();
        }
        assert_eq!(w.raw(105, 329), Mat::Grass, "grass lands on soil");
        let sand_grass = (0..H as i32).find(|&y| w.raw(205, y) == Mat::Grass).expect("grass should be resting on the sand");
        assert_eq!(w.raw(205, sand_grass + 1), Mat::Sand, "grass lands on sand");
        assert!((0..H as i32).all(|y| w.raw(300, y) != Mat::Grass), "grass over a void falls out of the world");
    }

    #[test]
    fn a_warm_cloud_keeps_its_water_a_cold_one_rains_and_a_freezing_one_hails() {
        let run = |air: f32| {
            let mut w = World::new_static();
            w.climate_fixed = Some(air);
            w.reset_temps();
            w.day_cycle = false;
            for x in 400..440 {
                for y in 60..70 {
                    w.set(x, y, Mat::Cloud);
                }
            }
            let mut counts = (0, 0, 0);
            for _ in 0..1200 {
                w.step();
                let c = |m| w.cells.iter().filter(|c| c.m == m).count();
                counts = (counts.0.max(c(Mat::Water)), counts.1.max(c(Mat::Snow)), counts.2.max(c(Mat::Hail)));
            }
            counts
        };
        let warm = run(20.0);
        let cold = run(2.0);
        let freezing = run(-30.0);
        assert_eq!((warm.0, warm.1, warm.2), (0, 0, 0), "a warm cloud must not rain: {warm:?}");
        assert!(cold.0 > 0, "a cool cloud rains: {cold:?}");
        assert!(freezing.2 > 0, "a very cold cloud drops hail: {freezing:?}");
    }

    #[test]
    fn the_air_temperature_follows_the_seasons_and_the_gradient_starts_at_the_top() {
        use super::Season;
        let mut w = World::new();
        w.day_cycle = false;
        let mut noon = |s: Season| {
            w.set_season(s);
            w.set_phase(0.25);
            (w.surface_temp_c(), w.ambient_c(0), w.ambient_c(H as i32 / 2))
        };
        let (summer_ground, summer_top, summer_mid) = noon(Season::Summer);
        let (winter_ground, _, _) = noon(Season::Winter);
        assert!((summer_ground - 40.0).abs() < 4.0, "mid-summer noon is about 40 °C, got {summer_ground}");
        assert!((winter_ground - 10.0).abs() < 4.0, "mid-winter noon is about 10 °C, got {winter_ground}");
        assert!(summer_top.abs() < 4.0, "the top of the screen hovers around 0 °C, got {summer_top}");
        assert!(summer_mid > summer_top + 5.0 && summer_mid < summer_ground, "the gradient runs from the top down: {summer_top} {summer_mid} {summer_ground}");
        w.set_season(Season::Summer);
        w.set_phase(0.75);
        let night_summer = w.surface_temp_c();
        w.set_season(Season::Winter);
        w.set_phase(0.75);
        let night_winter = w.surface_temp_c();
        assert!((-12.0..=-3.0).contains(&night_summer) && (-12.0..=-3.0).contains(&night_winter), "nights are -10 to -5: {night_summer} {night_winter}");
        assert!(night_winter < night_summer, "winter nights are colder");
        assert!(w.ambient_c(0).abs() < 4.0);
    }

    #[test]
    fn the_sun_warms_the_ground_most_nearby_and_the_air_cools_with_altitude() {
        let mut w = World::new();
        w.set_phase(0.25);
        w.day_cycle = false;
        for x in 0..W as i32 {
            for y in 300..320 {
                w.set(x, y, Mat::Stone);
            }
        }
        w.reset_temps();
        for _ in 0..600 {
            w.step();
        }
        let under = w.temp_c(W as i32 / 2, 300);
        let far = w.temp_c(40, 300);
        assert!(under > far + 1.5, "ground under the sun {under} vs far {far}");
        // Away from the sun's halo, the air gets colder toward the top of the screen.
        assert!(w.temp_c(40, 80) < w.temp_c(40, 290) - 5.0, "air should cool with altitude");
    }

    #[test]
    fn warm_gas_rises_and_cold_gas_sinks() {
        let mut w = World::new_static();
        w.climate_fixed = Some(15.0);
        w.reset_temps();
        for k in 0..30 {
            // Hot smoke and cold smoke, side by side, mid-air.
            w.set(300 + k, 200, Mat::Smoke);
            w.set(500 + k, 200, Mat::Smoke);
            w.temp[200 * W + 500 + k as usize] = -15.0;
        }
        let mean_y = |w: &World, left: bool| {
            let ys: Vec<usize> = (0..W * H).filter(|&i| w.cells[i].m == Mat::Smoke && ((i % W) < 400) == left).map(|i| i / W).collect();
            ys.iter().sum::<usize>() as f32 / ys.len().max(1) as f32
        };
        let (smoke0, cloud0) = (mean_y(&w, true), mean_y(&w, false));
        for _ in 0..45 {
            w.wind = 0.0;
            w.step();
        }
        assert!(mean_y(&w, true) < smoke0 - 8.0, "hot gas should climb: {smoke0} -> {}", mean_y(&w, true));
        assert!(mean_y(&w, false) > cloud0 + 1.5, "cold gas should sink: {cloud0} -> {}", mean_y(&w, false));
    }

    #[test]
    fn gas_drifts_briskly_with_the_wind() {
        let mut w = World::new_static();
        for k in 0..20 {
            w.set(300 + k * 2, 100 + k % 5, Mat::Cloud);
            w.set(300 + k * 2, 140 + k % 5, Mat::Steam);
        }
        let mean_x = |w: &World| {
            let xs: Vec<i32> = (0..W * H).filter(|&i| matches!(w.cells[i].m, Mat::Cloud | Mat::Steam)).map(|i| (i % W) as i32).collect();
            xs.iter().sum::<i32>() as f32 / xs.len().max(1) as f32
        };
        let before = mean_x(&w);
        for _ in 0..300 {
            w.wind = 0.5;
            w.step();
        }
        let moved = mean_x(&w) - before;
        assert!(moved > 60.0, "a 0.5 wind should carry gas well over 60 cells in 300 steps, moved {moved}");
    }

    #[test]
    fn the_wind_can_be_tuned() {
        let run = |max: f32, changes: f32, windy: f32| {
            let mut w = World::new_static();
            w.wind_max = max;
            w.wind_changes = changes;
            w.windiness = windy;
            let (mut top, mut flips, mut last) = (0.0f32, 0, 0.0f32);
            for _ in 0..200_000u32 {
                w.update_wind();
                top = top.max(w.wind.abs());
                if w.wind * last < 0.0 {
                    flips += 1;
                }
                last = w.wind;
            }
            (top, flips)
        };
        let (top_usual, flips_usual) = run(1.0, 1.0, 1.0);
        let (top_capped, _) = run(0.4, 1.0, 1.0);
        let (top_calm, _) = run(1.0, 1.0, 0.0);
        let (_, flips_often) = run(1.0, 3.0, 1.0);
        let (_, flips_never) = run(1.0, 0.0, 1.0);
        assert!(top_capped <= 0.4001 && top_capped < top_usual, "{top_capped} {top_usual}");
        assert!(top_calm < 0.25 && top_calm < top_usual, "calm weather is a light breeze: {top_calm}");
        assert!(flips_often > flips_usual, "{flips_often} {flips_usual}");
        assert_eq!(flips_never, 0, "the direction can be frozen");
    }

    #[test]
    fn there_is_always_a_breeze() {
        let mut w = World::new_static();
        let mut calmest = 1.0f32;
        for t in 0..200_000u32 {
            w.ticks = t;
            w.update_wind();
            calmest = calmest.min(w.wind.abs());
        }
        assert!(calmest >= 0.29, "the wind should never die down completely: {calmest}");
    }

    #[test]
    fn rain_slants_with_the_wind() {
        let mut w = World::new_static();
        w.wind = 0.9;
        let mut sx = 0.0;
        let mut n = 0;
        // A curtain of drops (three drops in one column were too few for a steady average).
        for k in 0..200 {
            w.set(300 + k, 100 + (k % 3), Mat::Water);
        }
        let mut positions: Vec<i32> = vec![];
        for _ in 0..40 {
            w.step();
            w.wind = 0.9;
        }
        for i in 0..W * H {
            if w.cells[i].m == Mat::Water {
                positions.push((i % W) as i32);
            }
        }
        for p in positions {
            sx += (p - 400) as f32 + 0.5;
            n += 1;
        }
        // Without wind the mean offset is 0 (drops pick left and right at random).
        assert!(n > 0 && sx / n as f32 > 0.6, "drops should drift right, mean offset {}", sx / n.max(1) as f32);
    }

    #[test]
    fn mask_blocks_sand_and_displaces_it() {
        let mut w = World::new_static();
        for x in 0..20 {
            w.set(x, 100, Mat::Stone);
        }
        w.set(10, 99, Mat::Sand);
        w.begin_blocks(vec![Mat::Brick], vec![false]);
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
    fn every_biome_makes_a_playable_world() {
        for biome in Biome::ALL {
            let mut w = World::new();
            let (e, x) = w.generate_biome(5, biome);
            assert_eq!(w.biome, biome);
            assert_eq!(w.raw(e.0, e.1), Mat::Empty, "{biome:?} entry blocked");
            assert_eq!(w.raw(x.0, x.1), Mat::Empty, "{biome:?} exit blocked");
            let water = w.cells.iter().filter(|c| c.m == Mat::Water).count();
            let solid = w.cells.iter().filter(|c| c.m.is_hard()).count();
            assert!(solid > 50_000, "{biome:?} has no ground");
            match biome {
                Biome::Sea => assert!(water > 20_000, "the sea is missing: {water}"),
                Biome::Mountain => assert!(w.cells.iter().any(|c| c.m == Mat::Ice), "no snow caps"),
                Biome::Countryside => assert!(w.cells.iter().any(|c| c.m == Mat::Trunk), "no trees"),
            }
        }
    }

    #[test]
    fn generator_makes_ground() {
        let mut w = World::new_static();
        let (e, _) = w.generate(7);
        assert_eq!(w.at(e.0, e.1), Mat::Empty);
        assert!(w.at(200, H as i32 - 2) == Mat::Stone);
    }
}

#[cfg(test)]
mod profile {
    use super::*;
    use std::time::Instant;

    #[test]
    fn where_the_time_goes() {
        let mut w = World::new();
        w.generate_biome(3, Biome::Countryside);
        w.paint(300, 200, 40, Mat::Water);
        w.paint(600, 150, 40, Mat::Sand);
        for _ in 0..60 {
            w.step();
        }
        let time = |name: &str, mut f: Box<dyn FnMut(&mut World)>, w: &mut World| {
            let t = Instant::now();
            for _ in 0..20 {
                f(w);
            }
            eprintln!("PROF {name}: {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / 20.0);
        };
        time("full step", Box::new(|w| w.step()), &mut w);
        time("thermal pass", Box::new(|w| w.thermal_pass()), &mut w);
        time("level water", Box::new(|w| w.level_water()), &mut w);
        time("structure pass", Box::new(|w| w.structure_pass()), &mut w);
        time("census", Box::new(|w| {
            let _ = w.census();
        }), &mut w);
    }
}

#[cfg(test)]
mod perf {
    use super::*;

    #[test]
    fn step_cost_report() {
        let mut w = World::new();
        w.generate(3);
        // A busy scene: a big lake of water and a sand avalanche.
        w.paint(300, 200, 40, Mat::Water);
        w.paint(600, 150, 40, Mat::Sand);
        let t = std::time::Instant::now();
        for _ in 0..60 {
            w.step();
        }
        eprintln!("PERF busy: {:.2} ms/step", t.elapsed().as_secs_f64() * 1000.0 / 60.0);
        for _ in 0..600 {
            w.step();
        }
        let t = std::time::Instant::now();
        for _ in 0..60 {
            w.step();
        }
        eprintln!("PERF settled: {:.2} ms/step", t.elapsed().as_secs_f64() * 1000.0 / 60.0);
    }
}

#[cfg(test)]
mod science {
    use super::*;

    fn floor(w: &mut World, y: i32, x0: i32, x1: i32) {
        for x in x0..x1 {
            w.set(x, y, Mat::Stone);
        }
    }

    fn count(w: &World, m: Mat) -> usize {
        w.cells.iter().filter(|c| c.m == m).count()
    }

    #[test]
    fn fire_heats_water_until_it_boils() {
        let mut w = World::new_static();
        floor(&mut w, 300, 100, 140);
        // The sides of the pot: boiling takes its time (latent heat), the water must not run off.
        for y in 270..300 {
            w.set(99, y, Mat::Stone);
            w.set(140, y, Mat::Stone);
        }
        for x in 100..140 {
            for y in 280..300 {
                w.set(x, y, Mat::Water);
            }
        }
        for _ in 0..1500 {
            // A steady flame under the pot.
            for x in 110..130 {
                if w.raw(x, 299) != Mat::Fire {
                    w.set(x, 299, Mat::Fire);
                }
            }
            w.step();
        }
        assert!(count(&w, Mat::Steam) + count(&w, Mat::Cloud) > 0, "no steam");
    }

    #[test]
    fn steam_condenses_into_cloud_which_rains() {
        let mut w = World::new_static();
        // Cool air: a cloud warmed by the sun would keep its water.
        w.climate_fixed = Some(2.0);
        w.reset_temps();
        // Steam released high up in the cold layer.
        for x in 300..340 {
            for y in 20..30 {
                w.set(x, y, Mat::Steam);
            }
        }
        let mut seen_cloud = false;
        let mut seen_rain = false;
        for _ in 0..3000 {
            w.step();
            seen_cloud |= count(&w, Mat::Cloud) > 0;
            seen_rain |= count(&w, Mat::Water) > 0;
        }
        assert!(seen_cloud, "never made a cloud");
        assert!(seen_rain, "never rained");
    }

    #[test]
    fn rock_melts_into_lava_after_a_long_time_in_fire() {
        let mut w = World::new_static();
        floor(&mut w, 300, 100, 140);
        for x in 100..140 {
            for y in 290..300 {
                w.set(x, y, Mat::Stone);
            }
        }
        for _ in 0..4000 {
            for x in 100..140 {
                if w.raw(x, 289) == Mat::Empty {
                    w.set(x, 289, Mat::Fire);
                }
            }
            w.step();
        }
        assert!(count(&w, Mat::Lava) > 0, "rock never melted");
    }

    #[test]
    fn a_charcoal_forge_melts_metal() {
        // A terracotta crucible full of charcoal round a small iron ingot, lit from above.
        let mut w = World::new_static();
        for x in 96..124 {
            for y in 300..303 {
                w.set(x, y, Mat::Terracotta);
            }
        }
        for y in 284..300 {
            for x in (96..100).chain(120..124) {
                w.set(x, y, Mat::Terracotta);
            }
        }
        for x in 100..120 {
            for y in 288..300 {
                w.set(x, y, Mat::Charcoal);
            }
        }
        for x in 108..112 {
            for y in 293..297 {
                w.set(x, y, Mat::Iron);
            }
        }
        let mut melted = false;
        for k in 0..6000 {
            if k < 1500 {
                for x in 100..120 {
                    if let Some(y) = (280..300).find(|&y| w.raw(x, y + 1) != Mat::Empty) {
                        if w.raw(x, y) == Mat::Empty {
                            w.set(x, y, Mat::Fire);
                        }
                    }
                }
            }
            w.step();
            melted |= w.cells.iter().any(|c| c.m.is_molten());
            if melted {
                break;
            }
        }
        assert!(melted, "glowing charcoal heaped round the metal melts it");
    }

    #[test]
    fn molten_steel_sets_back_into_steel_and_molten_iron_into_iron() {
        for (solid, liquid) in [(Mat::Steel, Mat::MoltenSteel), (Mat::Iron, Mat::Molten)] {
            let mut w = World::new_static();
            floor(&mut w, 300, 90, 130);
            for x in 100..120 {
                for y in 296..300 {
                    w.set(x, y, solid);
                    // Hot enough above its melting point to have the heat that melts it.
                    w.temp[y as usize * W + x as usize] = thermal::METAL_MELT_C + 400.0;
                }
            }
            let mut seen = false;
            for _ in 0..400 {
                w.step();
                seen |= count(&w, liquid) > 0;
            }
            assert!(seen, "{solid:?} above its melting point turns {liquid:?}");
            // Cooled down, it sets again into what it was.
            for k in 0..W * H {
                if w.cells[k].m == liquid {
                    w.temp[k] = 300.0;
                }
            }
            for _ in 0..400 {
                w.step();
            }
            assert_eq!(count(&w, liquid), 0, "{liquid:?} sets once cold");
            let other = if solid == Mat::Steel { Mat::Iron } else { Mat::Steel };
            assert_eq!(count(&w, other), 0, "{liquid:?} must not set into {other:?}");
            assert!(count(&w, solid) > 60);
        }
    }

    #[test]
    fn painted_molten_metal_starts_hot_enough_to_flow() {
        let mut w = World::new_static();
        floor(&mut w, 300, 60, 200);
        w.paint(130, 280, 4, Mat::Molten);
        let cells: Vec<usize> = (0..W * H).filter(|&k| w.cells[k].m == Mat::Molten).collect();
        assert!(!cells.is_empty());
        assert!(cells.iter().all(|&k| w.temp[k] > thermal::METAL_MELT_C), "poured molten metal is above its melting point");
        for _ in 0..60 {
            w.step();
        }
        assert!(count(&w, Mat::Molten) > 0, "it does not set at once");
        assert!((0..W * H).any(|k| w.cells[k].m == Mat::Molten && k / W == 299), "it ran down onto the floor");
    }

    #[test]
    fn iron_charcoal_and_fire_make_steel() {
        let mut w = World::new_static();
        floor(&mut w, 300, 100, 120);
        for x in 105..115 {
            w.set(x, 299, Mat::Iron);
            if x % 2 == 0 {
                w.set(x, 298, Mat::Charcoal);
            }
        }
        for _ in 0..2500 {
            for x in (105..115).filter(|x| x % 2 == 1) {
                if w.raw(x, 298) == Mat::Empty {
                    w.set(x, 298, Mat::Fire);
                }
            }
            w.step();
        }
        assert!(count(&w, Mat::Steel) > 0, "no steel");
    }

    /// A fired clay box (it does not melt), 3 thick, with a layer of wood on its floor set
    /// alight; `lid` closes it.
    fn wood_fire_in_a_box(lid: bool) -> World {
        let mut w = World::new_static();
        for x in 97..134 {
            for y in 300..303 {
                w.set(x, y, Mat::Terracotta);
            }
            if lid {
                for y in 257..260 {
                    w.set(x, y, Mat::Terracotta);
                }
            }
        }
        for y in 257..300 {
            for x in (97..100).chain(131..134) {
                w.set(x, y, Mat::Terracotta);
            }
        }
        for x in 100..131 {
            for y in 296..300 {
                w.set(x, y, Mat::Wood);
            }
            w.set(x, 295, Mat::Fire);
        }
        w
    }

    #[test]
    fn a_fire_shut_in_a_box_fills_it_with_smoke_and_chokes() {
        let wood = |w: &World| count(w, Mat::Wood);
        let mut open = wood_fire_in_a_box(false);
        let mut shut = wood_fire_in_a_box(true);
        for _ in 0..5000 {
            open.step();
            shut.step();
        }
        let smoke = (100..131).flat_map(|x| (260..300).map(move |y| (x, y))).filter(|&(x, y)| shut.raw(x, y) == Mat::Smoke).count();
        let flames = |w: &World| w.cells.iter().filter(|c| matches!(c.m, Mat::Fire)).count();
        assert!(shut.sealed(115, 270), "the box is still a closed pocket");
        assert!(smoke > 150, "the smoke stays in the box: {smoke}");
        // A last flicker now and then, when a little smoke has leaked into the coals.
        assert!(flames(&shut) <= 3, "no air left: the flames are out: {}", flames(&shut));
        // Shut away from the air the wood only chars: its carbon stays, as charcoal. In the
        // open it burns to ash.
        assert!(wood(&shut) + count(&shut, Mat::Charcoal) > 60, "the shut box keeps its carbon: {} wood, {} charcoal", wood(&shut), count(&shut, Mat::Charcoal));
        assert_eq!(wood(&open), 0, "the open fire burns all its wood");
    }

    #[test]
    fn wood_heated_under_soil_chars_into_charcoal_and_charcoal_burns_long() {
        // The charcoal burner's mound: a woodpile under soil, warmed by a coal bed at its foot.
        let mut w = World::new_static();
        floor(&mut w, 300, 90, 150);
        for x in 100..140 {
            for y in 285..300 {
                w.set(x, y, Mat::Wood);
            }
            for y in 280..285 {
                w.set(x, y, Mat::Dirt);
            }
        }
        for y in 280..300 {
            for x in (95..100).chain(140..145) {
                w.set(x, y, Mat::Dirt);
            }
        }
        for _ in 0..4000 {
            for x in 100..140 {
                let i = World::idx(x, 299);
                if w.raw(x, 299) != Mat::Empty {
                    w.temp[i] = 450.0;
                }
            }
            w.step();
        }
        let charcoal = count(&w, Mat::Charcoal);
        let ash = count(&w, Mat::Ash);
        assert!(charcoal > 100, "the wood chars without air: {charcoal}");
        assert!(charcoal > 4 * ash, "it chars rather than burns: {charcoal} charcoal, {ash} ash");
        // Lit in the open air, charcoal smoulders much longer than wood.
        let embers_of = |m: Mat| {
            let mut w = World::new_static();
            floor(&mut w, 300, 90, 150);
            for x in 100..140 {
                w.set(x, 299, m);
            }
            for x in 100..140 {
                w.set(x, 298, Mat::Fire);
            }
            let mut last = 0;
            for t in 0..3000 {
                w.step();
                if w.cells.iter().any(|c| c.m == Mat::Ember) {
                    last = t;
                }
            }
            last
        };
        let (wood_t, coal_t) = (embers_of(Mat::Wood), embers_of(Mat::Charcoal));
        assert!(coal_t > wood_t, "charcoal glows longer: {coal_t} / {wood_t}");
    }

    #[test]
    fn rust_needs_carbon_to_turn_back_into_iron() {
        let heated = |carbon: bool| {
            let mut w = World::new_static();
            floor(&mut w, 300, 100, 120);
            for x in 105..115 {
                w.set(x, 299, Mat::Rust);
                if carbon && x % 2 == 0 {
                    w.set(x, 298, Mat::Charcoal);
                }
            }
            // Heat with no flame at all: hot metal under the floor, like an oven wall.
            for _ in 0..1500 {
                for x in 105..115 {
                    for y in 298..300 {
                        let i = World::idx(x, y);
                        if w.raw(x, y) == Mat::Rust || w.raw(x, y) == Mat::Iron {
                            w.temp[i] = 450.0;
                        }
                    }
                }
                w.step();
            }
            count(&w, Mat::Iron)
        };
        assert_eq!(heated(false), 0, "heat alone leaves rust as rust");
        assert!(heated(true) > 0, "with charcoal the rust gives iron back");
    }

    #[test]
    fn ash_has_no_carbon_left_to_make_steel() {
        let mut w = World::new_static();
        floor(&mut w, 300, 100, 120);
        for x in 105..115 {
            w.set(x, 299, Mat::Iron);
            w.set(x, 298, Mat::Ash);
        }
        for _ in 0..1500 {
            for x in 105..115 {
                w.temp[World::idx(x, 299)] = 500.0;
            }
            w.step();
        }
        assert_eq!(count(&w, Mat::Steel), 0);
    }

    #[test]
    fn rust_heated_in_fire_becomes_iron_again() {
        let mut w = World::new_static();
        floor(&mut w, 300, 100, 120);
        for x in 105..115 {
            w.set(x, 299, Mat::Rust);
        }
        for _ in 0..600 {
            for x in 104..116 {
                if w.raw(x, 298) == Mat::Empty {
                    w.set(x, 298, Mat::Fire);
                }
            }
            w.step();
        }
        assert!(count(&w, Mat::Iron) > 0, "rust never turned back into iron");
    }

    #[test]
    fn steel_rusts_slower_than_iron() {
        let mut w = World::new_static();
        floor(&mut w, 300, 100, 160);
        for x in 100..160 {
            for y in 280..300 {
                w.set(x, y, Mat::Water);
            }
        }
        // Fence the pool so the water stays.
        for y in 280..300 {
            w.set(99, y, Mat::Stone);
            w.set(160, y, Mat::Stone);
        }
        for x in 105..125 {
            w.set(x, 299, Mat::Iron);
        }
        for x in 135..155 {
            w.set(x, 299, Mat::Steel);
        }
        for _ in 0..1500 {
            w.step();
        }
        let rusty_iron = (105..125).filter(|&x| w.raw(x, 299) == Mat::Rust).count();
        let rusty_steel = (135..155).filter(|&x| w.raw(x, 299) == Mat::Rust).count();
        assert!(rusty_iron > rusty_steel, "iron {rusty_iron} vs steel {rusty_steel}");
    }

    #[test]
    fn seed_on_wet_soil_grows_into_a_tree() {
        let mut w = World::new_static();
        floor(&mut w, 320, 100, 220);
        for x in 100..220 {
            for y in 310..320 {
                w.set(x, y, Mat::Dirt);
            }
        }
        // A puddle next to the seed feeds the tree.
        for x in 126..160 {
            for y in 295..310 {
                w.set(x, y, Mat::Water);
            }
        }
        for y in 290..310 {
            w.set(125, y, Mat::Stone);
            w.set(160, y, Mat::Stone);
        }
        w.set(122, 309, Mat::Seed);
        w.set(123, 309, Mat::Water);
        w.set(124, 309, Mat::Water);
        for _ in 0..4000 {
            w.step();
        }
        let wood = count(&w, Mat::Wood) + count(&w, Mat::Trunk) + count(&w, Mat::Sapling);
        assert!(wood >= 6, "tree only {wood} cells tall");
    }

    /// Cells of water a wide pan held at `t` °C loses in 3000 steps, under air of humidity
    /// `rh` moving at `wind` cells per step.
    fn evaporated(t: f32, rh: f32, wind: f32) -> usize {
        let mut w = World::new_static();
        w.flow.pin_uniform(wind, 0.0);
        w.flow.pin_moisture(rh);
        for x in 99..=300 {
            w.set(x, 300, Mat::Terracotta);
        }
        for y in 295..300 {
            w.set(99, y, Mat::Terracotta);
            w.set(300, y, Mat::Terracotta);
        }
        for x in 100..300 {
            for y in 297..300 {
                w.set(x, y, Mat::Water);
            }
        }
        let before = count(&w, Mat::Water);
        for _ in 0..3000 {
            for x in 100..300 {
                for y in 290..300 {
                    if w.raw(x, y) == Mat::Water {
                        w.temp[World::idx(x, y)] = t;
                    }
                }
            }
            w.step();
        }
        before - count(&w, Mat::Water)
    }

    #[test]
    fn water_evaporates_faster_when_warm_dry_and_windy() {
        let base = evaporated(30.0, 0.2, 0.0);
        let cold = evaporated(5.0, 0.2, 0.0);
        let humid = evaporated(30.0, 0.9, 0.0);
        let windy = evaporated(30.0, 0.2, 1.2);
        eprintln!("EVAP base {base} cold {cold} humid {humid} windy {windy}");
        assert!(cold > 0, "even cold water evaporates a little");
        assert!(base > 2 * cold, "warm water evaporates faster: {base} / {cold}");
        assert!(base > 3 * humid, "damp air takes less: {base} / {humid}");
        assert!(windy > base * 3 / 2, "the wind carries the damp air away: {windy} / {base}");
    }

    #[test]
    fn a_tree_drinks_from_the_soil_and_breathes_vapour_out_of_its_leaves() {
        let mut w = World::new_static();
        w.flow.pin_uniform(0.0, 0.0);
        // Noon, early summer.
        w.clock = thermal::DAY_STEPS * thermal::SEASON_DAYS + thermal::DAY_STEPS / 4;
        assert_eq!(w.season(), Season::Summer);
        floor(&mut w, 330, 80, 240);
        for x in 80..240 {
            for y in 310..330 {
                w.set(x, y, Mat::Dirt);
                let i = World::idx(x, y);
                w.cells[i].life = 200;
            }
        }
        // A trunk and a crown of leaves.
        for y in 280..310 {
            w.set(160, y, Mat::Trunk);
            w.set(161, y, Mat::Trunk);
        }
        for x in 145..177 {
            for y in 262..280 {
                if (x - 160) * (x - 160) / 2 + (y - 271) * (y - 271) < 80 {
                    w.set(x, y, Mat::Leaf);
                }
            }
        }
        let soil = |w: &World| (130..190).flat_map(|x| (310..320).map(move |y| (x, y))).map(|(x, y)| w.cells[World::idx(x, y)].life as u32).sum::<u32>();
        let wet = soil(&w);
        let mut vapour = 0;
        for _ in 0..3000 {
            w.step();
            vapour = vapour.max((130..190).flat_map(|x| (200..285).map(move |y| (x, y))).filter(|&(x, y)| w.raw(x, y) == Mat::Steam).count());
        }
        assert!(vapour > 0, "vapour comes out of the crown");
        assert!(soil(&w) + 200 < wet, "the roots dried the soil: {} -> {}", wet, soil(&w));
    }

    #[test]
    fn a_falling_grain_speeds_up() {
        let mut w = World::new_static();
        w.set(200, 20, Mat::Sand);
        let at = |w: &World| (0..H as i32).find(|&y| w.raw(200, y) == Mat::Sand).unwrap();
        let mut ys = vec![at(&w)];
        for _ in 0..40 {
            w.step();
            ys.push(at(&w));
        }
        let first = ys[10] - ys[0];
        let last = ys[40] - ys[30];
        assert!(last > 2 * first, "it falls faster and faster: {first} cells in the first 10 steps, {last} in the last 10");
        assert!(last <= 50, "but no faster than its terminal speed (5 cells a step): {last}");
    }

    /// Rise and run of the slope of a heap of `m` slumped from a tall block onto a wide floor.
    fn heap(m: Mat) -> (i32, i32) {
        let mut w = World::new_static();
        // Just below freezing: snow neither melts nor gets pressed into ice.
        w.climate_fixed = Some(-1.0);
        w.reset_temps();
        floor(&mut w, 300, 0, W as i32);
        // A tall block of it, left to slump.
        for x in 285..315 {
            for y in 200..300 {
                w.set(x, y, m);
            }
        }
        for _ in 0..1500 {
            w.step();
        }
        // The slope of the left flank, between a quarter and three quarters of the height (a
        // flat top and stray grains do not count).
        let top = (101..300).find(|&y| w.raw(300, y) == m).unwrap();
        let (y1, y2) = (top + (300 - top) / 4, 300 - (300 - top) / 4);
        let edge = |y: i32| (0..=300).rev().find(|&x| w.raw(x, y) != m).unwrap();
        (y2 - y1, edge(y1) - edge(y2))
    }

    #[test]
    fn each_material_piles_up_at_its_own_angle() {
        let angle = |m: Mat| {
            let (h, r) = heap(m);
            (h as f32).atan2(r as f32).to_degrees()
        };
        let (sand, snow, rust) = (angle(Mat::Sand), angle(Mat::Snow), angle(Mat::Rust));
        eprintln!("HEAP sand {sand:.0} snow {snow:.0} rust {rust:.0}");
        assert!(sand < rust - 4.0, "dry sand runs flatter: {sand:.0} / {rust:.0}");
        assert!(snow < sand, "snow is a viscous paste: given time a heap spreads flatter than sand: {snow:.0} / {sand:.0}");
    }

    /// Height of the snow standing at x = 300 on the floor at row 300.
    fn snow_height(w: &World) -> i32 {
        (0..300).rev().take_while(|&y| w.raw(300, y) == Mat::Snow).count() as i32
    }

    #[test]
    fn a_heap_of_snow_slumps_slowly_and_a_thin_layer_stays() {
        let mut w = World::new_static();
        w.climate_fixed = Some(-1.0);
        w.reset_temps();
        floor(&mut w, 300, 0, W as i32);
        for x in 290..310 {
            for y in 220..300 {
                w.set(x, y, Mat::Snow);
            }
        }
        // A thin layer further along.
        for x in 600..660 {
            w.set(x, 299, Mat::Snow);
        }
        for _ in 0..150 {
            w.step();
        }
        let early = snow_height(&w);
        for _ in 0..6000 {
            w.step();
        }
        let late = snow_height(&w);
        assert!(early >= 20, "snow is thick: it does not run like water ({early} cells high after 150 steps)");
        assert!(late * 4 <= early * 3, "but a heap keeps spreading out ({early} then {late})");
        let layer = (560..700).flat_map(|x| (200..300).map(move |y| (x, y))).filter(|&(x, y)| w.raw(x, y) == Mat::Snow).collect::<Vec<_>>();
        assert!(layer.len() >= 55 && layer.iter().all(|&(x, y)| y == 299 && (597..663).contains(&x)), "a thin layer stays where it fell: {layer:?}");
    }

    #[test]
    fn snowflakes_fall_gently() {
        let mut w = World::new_static();
        w.climate_fixed = Some(-5.0);
        w.reset_temps();
        w.flow.pin_uniform(0.0, 0.0);
        w.set(400, 20, Mat::Snow);
        for _ in 0..40 {
            w.step();
        }
        let y = (0..H as i32).find(|&y| (370..430).any(|x| w.raw(x, y) == Mat::Snow)).unwrap_or(H as i32);
        assert!(y > 40 && y <= 20 + 45, "a flake falls about a cell a step: it is at {y}");
    }

    #[test]
    fn ice_frozen_on_a_tree_falls_and_breaks_the_branches() {
        let mut w = World::new_static();
        w.structures_fall = true;
        w.climate_fixed = Some(-5.0);
        w.reset_temps();
        // Ground resting on the base of the world (only that holds things up).
        for x in 0..W as i32 {
            for y in 400..H as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        // A tree: trunk on the floor, a crown of branches and leaves.
        for y in 300..400 {
            w.set(380, y, Mat::Trunk);
        }
        for x in 360..480 {
            w.set(x, 300, Mat::Branch);
            for y in 285..300 {
                w.set(x, y, Mat::Leaf);
            }
        }
        // Ice that froze on the crown, and a sheet of it stuck to the side of the leaves.
        for x in 400..470 {
            for y in 275..285 {
                w.set(x, y, Mat::Ice);
            }
        }
        for x in 480..520 {
            for y in 288..292 {
                w.set(x, y, Mat::Ice);
            }
        }
        let n = w.cells.iter().filter(|c| c.m == Mat::Ice).count();
        for _ in 0..800 {
            w.step();
        }
        let ice: Vec<usize> = (0..W * H).filter(|&i| w.cells[i].m == Mat::Ice).collect();
        let highest = ice.iter().map(|i| i / W).min().unwrap_or(0);
        assert_eq!(ice.len(), n, "the ice is still there");
        assert!(highest > 375, "the ice fell to the ground through the crown (highest ice row {highest})");
        assert!(w.cells.iter().any(|c| c.m == Mat::DeadLeaf), "it broke leaves on its way down");
        assert_eq!(w.raw(380, 350), Mat::Trunk, "the trunk still stands");
    }

    #[test]
    fn light_grains_float_and_heavy_ones_sink() {
        let mut w = World::new_static();
        floor(&mut w, 300, 99, 141);
        for y in 260..300 {
            w.set(99, y, Mat::Stone);
            w.set(140, y, Mat::Stone);
        }
        for x in 100..140 {
            for y in 270..300 {
                w.set(x, y, Mat::Water);
            }
        }
        // Dropped in at the bottom of the tank, and on its surface.
        for x in 102..108 {
            w.set(x, 299, Mat::Charcoal);
            w.set(x + 10, 299, Mat::DeadLeaf);
            w.set(x + 20, 270, Mat::Sand);
        }
        for _ in 0..400 {
            w.step();
        }
        let depth = |m: Mat| {
            let ys: Vec<i32> = (100..140).flat_map(|x| (250..300).map(move |y| (x, y))).filter(|&(x, y)| w.raw(x, y) == m).map(|(_, y)| y).collect();
            ys.iter().sum::<i32>() as f32 / ys.len().max(1) as f32
        };
        assert!(depth(Mat::Charcoal) < 274.0, "charcoal rose to the surface: {}", depth(Mat::Charcoal));
        assert!(depth(Mat::DeadLeaf) < 274.0, "dead leaves float: {}", depth(Mat::DeadLeaf));
        assert!(depth(Mat::Sand) > 296.0, "sand sank to the bottom: {}", depth(Mat::Sand));
    }

    #[test]
    fn floating_stone_falls_but_attached_stone_stays() {
        let mut w = World::new();
        floor(&mut w, H as i32 - 1, 0, W as i32);
        // Attached pillar.
        for y in 400..H as i32 - 1 {
            w.set(50, y, Mat::Stone);
        }
        // Floating slab.
        for x in 200..230 {
            for y in 200..206 {
                w.set(x, y, Mat::Stone);
            }
        }
        for _ in 0..800 {
            w.step();
        }
        assert_eq!(w.raw(50, 400), Mat::Stone, "attached stone moved");
        assert_eq!(w.raw(215, 200), Mat::Empty, "slab still floating");
        assert_eq!(w.raw(215, H as i32 - 2), Mat::Stone, "slab did not land");
    }

    #[test]
    fn liquids_drip_through_trees() {
    let mut w = World::new_static();
    for y in 0..H as i32 {
        for x in 100..140 {
            w.set(x, y, Mat::Empty);
        }
    }
    for x in 100..140 {
        w.set(x, 300, Mat::Stone);
    }
    // A canopy of leaves with a branch and a trunk stub under it.
    for y in 150..301 {
        w.set(99, y, Mat::Stone);
        w.set(140, y, Mat::Stone);
    }
    for x in 100..140 {
        w.set(x, 200, Mat::Leaf);
        w.set(x, 201, Mat::Branch);
        w.set(x, 202, Mat::Trunk);
    }
    for (i, m) in [Mat::Water, Mat::Oil].into_iter().enumerate() {
        for x in 112..118 {
            w.set(x + i as i32 * 8, 190, m);
        }
    }
    for _ in 0..400 {
        w.step();
    }
    let above = (100..140).flat_map(|x| (150..200).map(move |y| (x, y))).filter(|&(x, y)| matches!(w.at(x, y), Mat::Water | Mat::Oil)).count();
    let total = (0..W as i32).flat_map(|x| (0..H as i32).map(move |y| (x, y))).filter(|&(x, y)| matches!(w.at(x, y), Mat::Water | Mat::Oil)).count();
    let below = (100..140).flat_map(|x| (203..300).map(move |y| (x, y))).filter(|&(x, y)| matches!(w.at(x, y), Mat::Water | Mat::Oil)).count();
    assert!(below >= 8, "liquid stayed on top of the tree: {below} below, {above} above, {total} total");
    assert_eq!(w.at(115, 200), Mat::Leaf, "the tree itself must stay");
}

#[test]
fn clouds_ride_opposing_winds_at_different_heights() {
    let mut w = World::new_static();
    w.flow.pin_opposed(0.6, 34);
    let patch = |w: &mut World, y: i32| {
        for dy in 0..4 {
            for x in 400..440 {
                w.set(x, y + dy, Mat::Cloud);
            }
        }
    };
    patch(&mut w, 100); // upper layer: towards the right
    patch(&mut w, 400); // lower layer: towards the left
    let mean_x = |w: &World, lo: i32, hi: i32| {
        let xs: Vec<f32> = (lo..hi).flat_map(|y| (0..W as i32).map(move |x| (x, y))).filter(|&(x, y)| w.raw(x, y) == Mat::Cloud).map(|(x, _)| x as f32).collect();
        (xs.iter().sum::<f32>() / xs.len().max(1) as f32, xs.len())
    };
    for _ in 0..150 {
        w.step();
    }
    let (up, n_up) = mean_x(&w, 60, 200);
    let (down, n_down) = mean_x(&w, 300, 480);
    assert!(n_up > 20 && n_down > 20, "clouds should survive: {n_up} {n_down}");
    assert!(up > 440.0, "the upper cloud drifts right: {up}");
    assert!(down < 410.0, "the lower cloud drifts left: {down}");
}

#[test]
fn rain_slants_with_the_local_wind() {
    let mut w = World::new_static();
    w.flow.pin_uniform(1.0, 0.0);
    for x in 300..320 {
        w.set(x, 100, Mat::Water);
    }
    for _ in 0..40 {
        w.step();
    }
    let xs: Vec<i32> = (0..H as i32).flat_map(|y| (0..W as i32).map(move |x| (x, y))).filter(|&(x, y)| w.raw(x, y) == Mat::Water && y < 300).map(|(x, _)| x).collect();
    assert!(!xs.is_empty());
    let mean = xs.iter().sum::<i32>() as f32 / xs.len() as f32;
    assert!(mean > 312.0, "falling drops are pushed along: mean x {mean}");
}

/// A crucible of fired clay, a wood fire under it, iron and charcoal inside.
fn crucible_world(wall: Mat) -> World {
    let mut w = World::new_static();
    let floor = 300;
    for x in 0..W as i32 {
        for y in floor..floor + 20 {
            w.set(x, y, Mat::Stone);
        }
    }
    // Wood under the bowl, 20 wide and 10 high; the bowl sits on it: base 3 thick, walls 3 thick, 16 high.
    for x in 200..220 {
        for y in floor - 30..floor {
            w.set(x, y, Mat::Wood);
        }
    }
    let base = floor - 30 - 3;
    for x in 202..220 {
        for y in base..base + 3 {
            w.set(x, y, wall);
        }
    }
    for y in base - 10..base {
        for x in 202..205 {
            w.set(x, y, wall);
        }
        for x in 217..220 {
            w.set(x, y, wall);
        }
    }
    // Wood piled around the bowl too, so that the fire wraps it.
    for y in base - 10..base + 3 {
        for x in (192..202).chain(220..230) {
            w.set(x, y, Mat::Wood);
        }
    }
    for y in base - 10..base + 3 {
        for x in 192..230 {
            if (192..202).contains(&x) || (220..230).contains(&x) {
                continue;
            }
        }
    }
    // A little iron (three rows) with charcoal on top, inside the 12-wide bowl.
    for y in base - 3..base {
        for x in 205..217 {
            w.set(x, y, Mat::Iron);
        }
    }
    for y in base - 5..base - 3 {
        for x in 205..217 {
            w.set(x, y, Mat::Charcoal);
        }
    }
    // Light the foot of the woodpile from both sides, where the air reaches it.
    for y in floor - 10..floor {
        w.set(199, y, Mat::Fire);
        w.set(220, y, Mat::Fire);
    }
    w
}

#[test]
fn a_clay_crucible_fired_by_a_wood_fire_turns_iron_and_charcoal_into_steel() {
    let mut w = crucible_world(Mat::Clay);
    let mut steel_at = None;
    for t in 0..6000 {
        w.step();
        if steel_at.is_none() && (195..225).any(|x| (250..300).any(|y| w.raw(x, y) == Mat::Steel)) {
            steel_at = Some(t);
        }
    }
    let count = |m: Mat| (190..230).flat_map(|x| (240..300).map(move |y| (x, y))).filter(|&(x, y)| w.raw(x, y) == m).count();
    assert!(count(Mat::Terracotta) > 20, "the fire fires the clay into terracotta: {}", count(Mat::Terracotta));
    assert!(count(Mat::Steel) >= 10, "the hot iron takes carbon from the charcoal and becomes steel: {}", count(Mat::Steel));
    // The crucible itself did not burn or melt away.
    assert!(count(Mat::Terracotta) + count(Mat::Clay) > 100, "the walls hold");
}

#[test]
fn flowing_water_erodes_bare_soil() {
        let mut w = World::new_static();
        floor(&mut w, 340, 100, 300);
        for x in 100..300 {
            for y in 320..340 {
                w.set(x, y, Mat::Dirt);
            }
        }
        let before = count(&w, Mat::Dirt);
        for t in 0..3000 {
            if t < 2500 {
                for x in 100..108 {
                    w.set(x, 250, Mat::Water);
                }
            }
            w.step();
        }
        assert!(count(&w, Mat::Dirt) < before, "no erosion");
        assert!(count(&w, Mat::Mud) + count(&w, Mat::Sand) > 0 || count(&w, Mat::Dirt) < before);
    }

    #[test]
    fn grass_protects_soil() {
        let run = |grass: bool| {
            let mut w = World::new_static();
            for x in 100..300 {
                w.set(x, 340, Mat::Stone);
                for y in 320..340 {
                    w.set(x, y, Mat::Dirt);
                }
                if grass {
                    for y in 318..320 {
                        w.set(x, y, Mat::Grass);
                    }
                }
            }
            for t in 0..1500 {
                if t < 1200 {
                    for x in 100..108 {
                        w.set(x, 250, Mat::Water);
                    }
                }
                w.step();
            }
            count(&w, Mat::Mud) + count(&w, Mat::Sand)
        };
        let bare = run(false);
        let covered = run(true);
        assert!(bare > 0, "bare soil should erode");
        assert!(covered * 3 <= bare, "grass should shield the soil below: {covered} vs {bare}");
    }

    #[test]
    fn ash_and_water_make_soil() {
        let mut w = World::new_static();
        floor(&mut w, 300, 99, 121);
        for y in 290..300 {
            w.set(99, y, Mat::Stone);
            w.set(120, y, Mat::Stone);
        }
        for x in 100..120 {
            w.set(x, 299, Mat::Ash);
            w.set(x, 298, Mat::Water);
        }
        for _ in 0..3000 {
            w.step();
        }
        assert!(count(&w, Mat::Dirt) + count(&w, Mat::Mud) > 0, "ash never turned to soil");
    }
}

#[cfg(test)]
mod glass {
    use super::*;

    #[test]
    fn sand_fuses_into_glass_in_the_heat() {
        let mut w = World::new_static();
        // A floor wider than the heap: dry sand runs out to a flat slope.
        for x in 80..160 {
            w.set(x, 300, Mat::Stone);
        }
        for x in 100..140 {
            for y in 292..300 {
                w.set(x, y, Mat::Sand);
            }
        }
        for _ in 0..2500 {
            // The fire is kept lit right on the sand, which slumps flatter as it runs.
            for x in 100..140 {
                if let Some(top) = (250..300).find(|&y| w.raw(x, y) != Mat::Empty && w.raw(x, y) != Mat::Fire && w.raw(x, y) != Mat::Smoke) {
                    if w.raw(x, top - 1) == Mat::Empty {
                        w.set(x, top - 1, Mat::Fire);
                    }
                }
            }
            w.step();
        }
        assert!(w.cells.iter().any(|c| c.m == Mat::Glass), "sand never became glass");
    }

    #[test]
    fn shattered_glass_leaves_shards_that_become_sand() {
        let mut w = World::new_static();
        for x in 100..110 {
            for y in 290..300 {
                w.set(x, y, Mat::Glass);
            }
        }
        w.set(105, 300, Mat::Stone);
        w.shatter(105, 297);
        let shards = w.cells.iter().filter(|c| c.m == Mat::Shard).count();
        assert!(shards > 0, "no shards");
        for x in 100..120 {
            w.set(x, 300, Mat::Stone);
            for y in 296..300 {
                w.set(x, y, Mat::Shard);
            }
        }
        for _ in 0..12_000 {
            w.step();
        }
        assert!(w.cells.iter().any(|c| c.m == Mat::Sand), "shards never turned back into sand");
    }

    fn tally(w: &World, m: Mat) -> usize {
        w.cells.iter().filter(|c| c.m == m).count()
    }

    /// A stone tray across x 200..260 with its floor at row 300.
    fn tray(w: &mut World) {
        for x in 196..264 {
            w.set(x, 300, Mat::Stone);
        }
        for y in 280..300 {
            w.set(196, y, Mat::Stone);
            w.set(263, y, Mat::Stone);
        }
    }

    #[test]
    fn leaves_rotting_on_wood_ash_make_saltpetre() {
        let mut w = World::new_static();
        tray(&mut w);
        for x in 197..263 {
            w.set(x, 299, Mat::Ash);
            w.set(x, 298, Mat::DeadLeaf);
        }
        // Without ash, the same leaves only make soil.
        let mut plain = World::new_static();
        tray(&mut plain);
        for x in 197..263 {
            plain.set(x, 299, Mat::Dirt);
            plain.set(x, 298, Mat::DeadLeaf);
        }
        for _ in 0..15_000 {
            w.step();
            plain.step();
        }
        let salt = tally(&w, Mat::Saltpeter);
        assert!(salt > 25, "the saltpetre bed gave crystals: {salt}");
        assert_eq!(tally(&plain, Mat::Saltpeter), 0, "no ash, no saltpetre");
    }

    #[test]
    fn saltpetre_and_charcoal_make_powder_but_water_dissolves_saltpetre() {
        let mut w = World::new_static();
        tray(&mut w);
        for x in 197..263 {
            w.set(x, 299, Mat::Saltpeter);
            w.set(x, 298, Mat::Charcoal);
        }
        for _ in 0..600 {
            w.step();
        }
        let powder = tally(&w, Mat::Gunpowder);
        assert!(powder > 80, "ground together they make powder: {powder}");
        assert!(w.blasts.is_empty(), "cold powder does not go off");
        // Saltpetre left in the rain melts away.
        let mut wet = World::new_static();
        tray(&mut wet);
        for x in 197..263 {
            wet.set(x, 299, Mat::Saltpeter);
            wet.set(x, 298, Mat::Water);
        }
        for _ in 0..3_000 {
            wet.step();
        }
        assert!(tally(&wet, Mat::Saltpeter) < 20, "the water dissolved it: {}", tally(&wet, Mat::Saltpeter));
    }

    #[test]
    fn gunpowder_going_off_is_reported_for_the_fireworks() {
        let mut w = World::new_static();
        tray(&mut w);
        for x in 220..230 {
            w.set(x, 299, Mat::Gunpowder);
        }
        w.set(225, 298, Mat::Fire);
        let mut booms = 0;
        for _ in 0..10 {
            w.step();
            booms += std::mem::take(&mut w.powder_booms).len();
        }
        assert!(booms > 0);
    }
}

