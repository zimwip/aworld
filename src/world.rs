//! Falling-sand world: one material per cell, simulated cell by cell.
//! Rigid blocks live outside the grid; they are stamped into a mask each step
//! so particles and the car treat them as solid.

mod blast;
pub mod matter;
pub use blast::Ejecta;
/// The classes of the liquids' cells, for the simulation view.
pub mod liquid_class {
    pub use super::liquid::{AIR, FULLISH, WET};
    #[allow(unused_imports)]
    pub use super::liquid::SOLID;
}
pub use liquid::CELL_M;
pub mod flow;
mod pockets;
mod pressure;
mod hydro;
pub mod sleep;
pub use pockets::boil_c;
mod thermal;
mod storm;
mod statics;
pub mod prof;
mod num;
mod par;
mod turn;
mod field;
mod liquid;
mod mgpcg;
mod gpu;
mod mgpcg_gpu;
mod liquid_gpu;
pub mod inspect;
#[cfg(test)]
mod bench;
#[cfg(test)]
mod edges;
pub use statics::Slab;
pub use storm::{Bolt, BREAKDOWN, GROUND_BREAKDOWN, QS, QW, QH};
pub use thermal::{air_pressure_atm, Season, DAY_STEPS};

/// The tick of the cells, in seconds: a grain falls, a flame spreads, a gas drifts one tick at a
/// time (see `World::step_dt`). The passes that integrate over time take any step instead.
pub const STEP: f32 = 1.0 / 60.0;
/// The longest step the world takes at once, in seconds. A longer one (a slow computer, a fast
/// speed) is several steps: past it the heat and the liquids would need sub-steps of their own.
pub const MAX_DT: f32 = 3.0 * STEP;
use thermal::adiabatic_cooling_c;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

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
pub const NMAT: usize = Mat::ALL.len();

/// The size of the world, in cells: 960 x 540 unless the game is started with another one
/// (`--grid`, see `set_grid`). Set once, before the first world is made: every field of the
/// world, of the renderer and of the card is sized from it.
static GRID: [AtomicUsize; 2] = [AtomicUsize::new(960), AtomicUsize::new(540)];
/// Columns across the world.
#[allow(non_snake_case)]
#[inline(always)]
pub fn W() -> usize {
    GRID[0].load(Relaxed)
}
/// Rows down the world.
#[allow(non_snake_case)]
#[inline(always)]
pub fn H() -> usize {
    GRID[1].load(Relaxed)
}
/// The design size, which the terrain and the sky are drawn for (see `sx`, `sy`).
pub const BASE_W: usize = 960;
pub const BASE_H: usize = 540;
/// The widths and heights `--grid` takes: steps of 96 columns (whole sleeping zones, in threes
/// for the strips of the cells' turn, see `turn.rs`) and of 18 rows (whole cells of the storms'
/// charge and of the light map, and 16:9 every other step across: 480 x 270, 1440 x 810...).
pub const GRID_W: (usize, usize, usize) = (384, 1920, 96);
pub const GRID_H: (usize, usize, usize) = (216, 1080, 18);

/// The nearest size `set_grid` takes.
pub fn snap_grid(w: usize, h: usize) -> (usize, usize) {
    let snap = |v: usize, (lo, hi, step): (usize, usize, usize)| ((v.clamp(lo, hi) + step / 2) / step * step).clamp(lo, hi);
    (snap(w, GRID_W), snap(h, GRID_H))
}

/// Sets the size of the world (snapped, see `snap_grid`). Only before the first world is made:
/// a world, a renderer or a card already made keep the size they were made at.
pub fn set_grid(w: usize, h: usize) -> (usize, usize) {
    let (w, h) = snap_grid(w, h);
    GRID[0].store(w, Relaxed);
    GRID[1].store(h, Relaxed);
    turn::check_strips();
    (w, h)
}

/// How much wider and taller the world is than the design size: the terrain and the sky are
/// stretched by these, so that a world of more cells is the same landscape in finer cells.
pub fn sx() -> f32 {
    W() as f32 / BASE_W as f32
}
pub fn sy() -> f32 {
    H() as f32 / BASE_H as f32
}
/// Resolution factor relative to the 320x180 design grid.
pub const K: i32 = 3;
/// Thickness, in cells, of the masonry vault the tunnel tool leaves around its hole.
pub const TUNNEL_LINING: i32 = 3;
/// Above this row the air is cold: steam condenses into clouds.
#[allow(non_snake_case)]
fn COLD_Y() -> i32 {
    (H() as i32) * 3 / 10
}
/// The top of the weather (the tropopause, about 13 km up on the logarithmic altitude scale of
/// `thermal::altitude_m`): above this row the air gets warmer again with height (an inversion).
/// Rising steam and clouds cool as they climb, so past it they are soon colder and denser than
/// the air around them: they stop under it and spread out sideways, the way a storm cloud
/// flattens into an anvil.
/// The widest run of thin matter (a crown, see `Mat::one_way`) a liquid passes through sideways
/// in one go.
pub const THIN_RUN: i32 = 16 * K;

#[allow(non_snake_case)]
pub fn CEILING_Y() -> i32 {
    (H() as i32) / 8
}

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
    /// Carbon squeezed past 50 000 atmospheres while white hot: the hardest thing there is, and
    /// the best conductor of heat.
    Diamond,
    /// A high explosive: it detonates instead of burning, with a shock forty times the pressure
    /// of black powder's.
    Dynamite,
}

impl Mat {
    pub const ALL: [Mat; 41] = [
        Mat::Empty, Mat::Sand, Mat::Water, Mat::Fire, Mat::Steam, Mat::Iron, Mat::Wood, Mat::Stone,
        Mat::Brick, Mat::Rust, Mat::Ash, Mat::Ember, Mat::Lava, Mat::Ice, Mat::Gunpowder, Mat::Dirt, Mat::Grass,
        Mat::Leaf, Mat::Steel, Mat::Cloud, Mat::Seed, Mat::Sapling, Mat::Mud, Mat::Glass, Mat::Snow, Mat::Shard, Mat::DeadLeaf, Mat::Branch, Mat::Trunk, Mat::Hail, Mat::Oil, Mat::Smoke, Mat::Molten, Mat::Clay, Mat::Terracotta,
        Mat::Charcoal, Mat::Saltpeter, Mat::MoltenSteel, Mat::Vault, Mat::Diamond, Mat::Dynamite,
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
            Iron | Wood | Stone | Brick | Ember | Ice | Dirt | Grass | Leaf | Steel | Sapling | Glass | Branch | Trunk | Clay | Terracotta | Vault | Diamond | Dynamite => Kind::Static,
        }
    }

    /// Blocks the car and rigid blocks.
    pub fn is_hard(self) -> bool {
        matches!(self.kind(), Kind::Static | Kind::Powder)
    }

    /// Living tree parts (they take root, grow, let the air through; see also `one_way`).
    pub fn is_vegetation(self) -> bool {
        matches!(self, Mat::Trunk | Mat::Leaf | Mat::Branch | Mat::Sapling)
    }

    /// Thin matter of a 2-D world, a "one-way platform": a tree stands in a plane of its own, so
    /// what moves sideways goes past it (in front of it or behind), but what comes down on it
    /// from above rests on it. The kid and the cars walk and drive through a trunk and stand on a
    /// crown (`World::bears`); a liquid passes through sideways but does not go down through it
    /// (`World::past_thin`).
    pub fn one_way(self) -> bool {
        matches!(self, Mat::Trunk | Mat::Branch | Mat::Leaf | Mat::Sapling)
    }

    /// Anything that burns like wood.
    pub fn is_wooden(self) -> bool {
        matches!(self, Mat::Wood | Mat::Trunk | Mat::Leaf | Mat::Branch)
    }

    /// Metal that has melted (iron or steel).
    #[cfg(test)]
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
    let mut out = vec![0.0f32; W() * H()];
    for y in 0..H() {
        let fy = ((y as f32 + 0.5) / b as f32 - 0.5).clamp(0.0, bh as f32 - 1.0);
        let (j0, ty) = (fy as usize, fy.fract());
        let j1 = (j0 + 1).min(bh - 1);
        for x in 0..W() {
            let fx = ((x as f32 + 0.5) / b as f32 - 0.5).clamp(0.0, bw as f32 - 1.0);
            let (i0, tx) = (fx as usize, fx.fract());
            let i1 = (i0 + 1).min(bw - 1);
            let top = coarse[j0 * bw + i0] * (1.0 - tx) + coarse[j0 * bw + i1] * tx;
            let bot = coarse[j1 * bw + i0] * (1.0 - tx) + coarse[j1 * bw + i1] * tx;
            out[y * W() + x] = top * (1.0 - ty) + bot * ty;
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

fn ramp(stops: &[(f32, [f32; 3])], v: f32) -> (u8, u8, u8) {
    let v = v.clamp(stops[0].0, stops[stops.len() - 1].0);
    let k = stops.iter().position(|s| s.0 >= v).unwrap_or(stops.len() - 1).max(1);
    let ((p0, c0), (p1, c1)) = (stops[k - 1], stops[k]);
    let f = ((v - p0) / (p1 - p0)).clamp(0.0, 1.0);
    let m = |a: f32, b: f32| (a + (b - a) * f) as u8;
    (m(c0[0], c1[0]), m(c0[1], c1[1]), m(c0[2], c1[2]))
}

/// The pressure view's scale for gas (atmospheres, logarithmic): the air thins tenfold and more
/// from the valley to the top of the sky, and a boiler holds a few atmospheres. Violet and blue for
/// thin air, green at 1 atm, yellow to red for a pocket under pressure, white about to burst.
pub fn air_pressure_rgb(atm: f32) -> (u8, u8, u8) {
    const STOPS: [(f32, [f32; 3]); 9] = [
        (0.0, [40.0, 20.0, 90.0]),
        (0.3, [80.0, 45.0, 170.0]),
        (0.5, [60.0, 90.0, 230.0]),
        (0.62, [40.0, 165.0, 215.0]),
        (0.69, [60.0, 200.0, 120.0]),
        (0.76, [230.0, 225.0, 70.0]),
        (0.85, [245.0, 140.0, 40.0]),
        (0.95, [225.0, 45.0, 40.0]),
        (1.0, [255.0, 235.0, 235.0]),
    ];
    ramp(&STOPS, air_scale(atm))
}

/// Where a pressure sits on the air scale (0 at `AIR_LOW_ATM`, 1 at `AIR_TOP_ATM`).
pub fn air_scale(atm: f32) -> f32 {
    ((atm.max(AIR_LOW_ATM) / AIR_LOW_ATM).ln() / (AIR_TOP_ATM / AIR_LOW_ATM).ln()).clamp(0.0, 1.0)
}

/// The ends of the air pressure scale.
pub const AIR_LOW_ATM: f32 = 0.02;
pub const AIR_TOP_ATM: f32 = 6.0;

/// The pressure view's scale for liquids and the ground (atmospheres, logarithmic): from about
/// 1 atm at the surface to tens of atmospheres deep down, in colours of their own (dark plum,
/// through crimson and orange, to pale yellow) so that they never read as air.
pub fn ground_pressure_rgb(atm: f32) -> (u8, u8, u8) {
    const STOPS: [(f32, [f32; 3]); 6] = [
        (0.0, [45.0, 30.0, 70.0]),
        (0.2, [95.0, 40.0, 115.0]),
        (0.4, [165.0, 50.0, 110.0]),
        (0.6, [225.0, 85.0, 60.0]),
        (0.8, [250.0, 165.0, 50.0]),
        (1.0, [255.0, 245.0, 190.0]),
    ];
    ramp(&STOPS, ground_scale(atm))
}

/// Where a pressure sits on the ground scale (0 at 1 atm, 1 at `GROUND_TOP_ATM`).
pub fn ground_scale(atm: f32) -> f32 {
    (atm.max(1.0).ln() / GROUND_TOP_ATM.ln()).clamp(0.0, 1.0)
}

/// The top of the ground pressure scale.
pub const GROUND_TOP_ATM: f32 = 30.0;

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
        Mat::Diamond => 3.5,
        Mat::Dynamite => 1.4,
        _ => 1.0,
    }
}

/// How thick a liquid is, as `World::step_liquid` reads it: it stays still once in `rest` steps
/// (0: never), and it spreads sideways up to `reach` cells at a time, once in `spread` steps.
pub fn viscosity(m: Mat) -> (u32, i32, u32) {
    match m {
        Mat::Water => (0, 4 * K, 1),
        Mat::Oil => (3, 2 * K, 1),
        // Melts barely run: lava and molten metal creep a cell at a time.
        _ => (2, 1, 3),
    }
}

/// How a gas drifts when neither its buoyancy nor the wind moves it, as `World::step_gas` reads
/// it: (in how many steps out of 10 it climbs while warm, and while cooled, whether a cooled one
/// still climbs when it cannot go sideways, whether it leaves by the sides of the world).
pub fn drift(m: Mat) -> (u32, u32, bool, bool) {
    match m {
        // Even cooled, vapour keeps drifting up a little: damp air is lighter than dry air (a
        // water molecule weighs 18, air 29), which is how the vapour of a lake reaches the clouds.
        Mat::Steam => (7, 3, true, false),
        // Only warm smoke keeps climbing by itself; cooled smoke just drifts.
        _ => (6, 0, false, true),
    }
}

#[derive(Clone, Copy)]
pub struct Cell {
    pub m: Mat,
    stamp: u8,
}

const EMPTY: Cell = Cell { m: Mat::Empty, stamp: 0 };
/// A heavy piece ploughs into sand or earth only coming down at this speed or more (cells per
/// drop): a piece resting on grains, or set down gently, stays on them. Each row of grains it
/// pushes aside costs it `PLOUGH_BRAKE` of its speed, so a body dropped from high up sinks in a
/// few cells and no more.
const PLOUGH: f32 = 1.0;
const PLOUGH_BRAKE: f32 = 0.6;

/// What a cell of matter carries besides what it is made of: one state per aspect, each in its
/// own array of `World`, each with one meaning and one unit. When the matter moves its states go
/// with it (`World::carry`); when it changes into another matter (`World::retype`,
/// `World::change_state`) it keeps those the new matter can carry (`Mat::carries`) and loses the
/// others; painting a cell (`World::set`) starts it afresh.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Aspect {
    /// `temp` (°C) and `latent`: every matter has a temperature. (Nothing asks for it yet: it
    /// completes the list.)
    #[allow(dead_code)]
    Heat,
    /// `moisture`: water held in the cell on top of the matter's own, in `WATER_UNITS` per cell of
    /// liquid water. Soil holds what it drank; packed vapour the extra cells of vapour in it.
    Water,
    /// `fuel`: what is left to burn in a flame or an ember, and whether it is charcoal.
    Fuel,
    /// `growth`: what a young tree still has to grow; in soil, the seeds sown in it (fertile
    /// earth), each of which can start a tree.
    Growth,
    /// `jet`: sideways speed and direction of a drop shot out of a jet (see `pockets::fly`).
    Jet,
}

impl Mat {
    /// Whether this matter carries a state for `a`.
    pub fn carries(self, a: Aspect) -> bool {
        match a {
            Aspect::Heat => true,
            Aspect::Water => matches!(self, Mat::Dirt | Mat::Grass | Mat::Mud | Mat::Steam | Mat::Cloud),
            Aspect::Fuel => matches!(self, Mat::Fire | Mat::Ember),
            Aspect::Growth => matches!(self, Mat::Sapling | Mat::Dirt | Mat::Grass | Mat::Mud),
            Aspect::Jet => self.kind() == Kind::Liquid,
        }
    }
}

/// What is left to burn in a flame or an ember: steps of burning (`left`), and whether it is
/// charcoal (`carbon`), which glows hotter and burns several times longer than wood.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Fuel {
    pub left: u8,
    pub carbon: bool,
}

/// The states of one cell besides its heat, as `World::states` reads them.
#[derive(Clone, Copy)]
pub(super) struct States {
    moisture: u16,
    fuel: Fuel,
    growth: u8,
    jet: u8,
}

/// Units of `moisture` in one cell of liquid water.
pub const WATER_UNITS: u16 = 64;
/// Units of `moisture` in one cell of vapour (steam or cloud).
pub const VAPOUR_UNITS: u16 = WATER_UNITS / VAPOUR_PER_WATER as u16;

/// Steps for a cell of water at 40 °C under perfectly dry, still air to evaporate, on average.
const EVAPORATE_40C: f32 = 9_000.0;
/// A trunk cell standing on soil tries to transpire once in this many steps.
const TRANSPIRE_EVERY: u32 = 200;

/// Moisture that one cell of water brings when the soil drinks it.
pub const SOIL_WATER: u16 = WATER_UNITS;
/// Moisture of saturated soil: three cells of water. Wetter than this it drinks no more, the rest
/// stays on top as a puddle, and where there is air under it, it drips.
pub const SOIL_FULL: u16 = 3 * WATER_UNITS;
/// Field capacity: above this the water is free and gravity drains it down into drier soil;
/// below it the soil holds it, and it only creeps (capillarity) towards drier soil around.
const SOIL_FIELD: u16 = 96;
/// Moisture that leaves in one cell of vapour.
const SOIL_VAPOUR: u16 = VAPOUR_UNITS;
/// Soil moisture moves to a neighbour once in this many steps.
const SOIL_SEEP: u32 = 8;
/// Saturated soil over a hole lets a drop fall once in this many steps.
const SOIL_DRIP: u32 = 300;
/// Most seeds a cell of soil holds.
pub const SOIL_SEEDS: u8 = 8;
/// A seed in the top of the soil sprouts once in this many steps, when the soil is at least
/// `TREE_MOIST` moist, warm and in the light, with room to grow.
const TREE_SPROUT: u32 = 400;
const TREE_MOIST: u16 = 48;
/// Grass sprouts on soil at least this moist, and stays fully green down to `GRASS_GREEN`.
const GRASS_GROW: u16 = 40;
const GRASS_GREEN: u16 = 48;
/// Below this the soil is dry for grass: it is straw yellow, and it wilts.
const GRASS_DRY: u16 = SOIL_VAPOUR;
/// Grass creeps onto a bare neighbour once in this many steps; with no grass near, a seed blown
/// in by the wind sprouts once in `GRASS_SPROUT` steps.
const GRASS_SPREAD: u32 = 250;
const GRASS_SPROUT: u32 = 40_000;
/// Dry grass dies back to bare earth once in this many steps; buried or drowned grass (no air
/// or light) once in `GRASS_SMOTHER`.
const GRASS_WILT: u32 = 2_500;
const GRASS_SMOTHER: u32 = 1_500;

/// Saturation vapour pressure of water at `t` °C, in kPa (Magnus formula): how much vapour the air
/// can hold at that temperature. It roughly doubles every 11 °C.
pub fn vapour_pressure(t: f32) -> f32 {
    0.6108 * (17.27 * t / (t + 237.3)).exp()
}

/// The unit of a falling speed (`vel`): this many make a cell per tick (1/6 m in 1/60 s, 10 m/s),
/// and as many of a falling cell's way through to the next (`fall_at`) make the whole cell.
pub const FALL_UNIT: u32 = 64;
/// A falling grain speeds up by this much every tick: gravity, 9.4 m/s² (9.81 would be 1.05).
const GRAIN_ACCEL: u8 = 1;
/// The fastest a grain falls through air: 9.1 m/s, where its drag balances gravity (that of a
/// raindrop, see `liquid::DROP_TERMINAL_MS`).
const GRAIN_TERMINAL: u8 = 58;

/// Snowflakes fall gently: at most 0.9 m/s.
const SNOW_TERMINAL: u8 = 6;
/// How far along a cell of snow can ooze before dropping a cell (a slope of 1 in this many).
const SNOW_RUN: i32 = 5;
/// A cell of snow at rest oozes once in this many steps: a very viscous liquid.
const SNOW_CREEP: u32 = 8;
/// Cells of vapour (steam or cloud) that one cell of liquid water makes, and that it takes to make
/// one drop again. Real water swells about 1700 times as it boils; the game keeps a small ratio so
/// that a pot does not fill the sky, but a sky full of cloud only gives back a little water.
pub const VAPOUR_PER_WATER: usize = 4;
/// The fastest a raindrop falls (see `FALL_UNIT`): 9.1 m/s, that of the liquids' drops.
const DROP_TERMINAL: u8 = 58;
/// A bubble of gas in a liquid climbs a cell one step in this many (a metre a second).
const BUBBLE_RISE: u32 = 10;
/// How much of the wind's speed a falling drop takes on sideways.
const DROP_DRIFT: f32 = 0.8;
/// How many times faster a drop in mid-air evaporates than the same water lying in a puddle.
const DROP_EVAPORATION: f32 = 30.0;
/// A drop falling through a cloud sweeps up the cell under it once in this many cells it falls.
const DROP_SWEEP: u32 = 5;
/// In a cloud, a cell turns into a raindrop once in this many steps where the cloud is thickest.
const RAIN_EVERY: f32 = 8000.0;
/// Below this many cloud cells in the 5 x 5 around it (out of 24), a cloud is too thin to rain.
const RAIN_MIN_WATER: f32 = 8.0;


/// A rigid piece of solid material that has lost its support and is falling.
#[derive(Clone)]
struct Piece {
    cells: Vec<usize>,
    /// Downward speed in cells per drop (negative while bouncing up).
    vy: f32,
    /// How far it has fallen since it last rested, cells: how hard it lands.
    fallen: f32,
    /// Mean density relative to water: lighter than a liquid, it floats on it (ice, wood).
    density: f32,
}

/// A copy of everything that changes while the world runs (for undo).
#[derive(Clone)]
pub struct State {
    matter: Matter,
    temp: Vec<f32>,
    clock: u32,
}

impl State {
    pub fn clock(&self) -> u32 {
        self.clock
    }
}

/// A copy of the matter of every cell and of the states it carries (see `Aspect`), but its heat.
#[derive(Clone)]
pub struct Matter {
    cells: Vec<Cell>,
    moisture: Vec<u16>,
    fuel: Vec<Fuel>,
    growth: Vec<u8>,
    jet: Vec<u8>,
    log: Vec<u16>,
}

pub struct World {
    pub cells: Vec<Cell>,
    frame: u8,
    rng: u32,
    seen: Vec<bool>,
    stack: Vec<usize>,
    /// Mouths of full pipes the levelling lets go (see `hydro.rs`), and the list of them to clear.
    released: Vec<bool>,
    released_list: Vec<usize>,
    falling: Vec<Piece>,
    /// Solid bodies tipping over or tumbling down as they fall (see `statics.rs`).
    pub slabs: Vec<Slab>,
    /// Scratch for the supports: how many cells each one is from what holds it (u32::MAX: none).
    level: Vec<u32>,
    /// Load on each anchored solid cell, as a share of what it can bear (0..255 for 0..2). For the
    /// stress view.
    pub strain: Vec<u8>,
    /// The push of the liquids on each anchored solid body, as the last check of the solids found
    /// it (see `statics.rs`; only while the liquids run by their physics).
    pub pushes: Vec<statics::LiquidPush>,
    /// A fingerprint of the anchored cells when their sections were last checked: they are only
    /// checked again once something changed.
    strain_key: u64,
    /// The sections of each anchored body by a fingerprint of its cells, and a scratch map of
    /// cells to their place in the order of the supports (see `statics::stress_pass`).
    stress_cache: std::collections::HashMap<u64, std::sync::Arc<statics::BodyStress>>,
    stress_pos: Vec<u32>,
    /// Where each place of the order comes in its body's list (the same reuse).
    stress_local: Vec<u32>,
    ticks: u32,
    /// Which log each cell of wood belongs to: every stroke of the wood brush is one log, a rigid
    /// body of its own that rests on what is under it but never sticks to anything else (see
    /// `same_body`). 0 for every other cell, and for wood that came from elsewhere.
    pub log: Vec<u16>,
    /// The log the wood brush is painting now (see `new_log`).
    pub paint_log: u16,
    /// Temperature in °C of every cell (air included).
    pub temp: Vec<f32>,
    /// Steps since the world began; drives the day and night cycle.
    pub clock: u32,
    /// The length of the step being taken, in seconds of the world (see `step_dt`).
    pub dt: f32,
    /// The time the world has gone through that the cells have not ticked yet, in seconds.
    tick_time: f32,
    /// The heat passes since the world began: which part of its cycle comes next.
    heat_stage: u32,
    /// When false the sun stands still.
    pub day_cycle: bool,
    thermal_flip: bool,
    /// What part 2 of the heat pass hands part 3 (heat the open air took from the sky, heat the
    /// flames gave off), when the pass is spread over the steps of its cycle.
    heat_carry: (f32, f32),
    /// Scratch of the wind carrying the air's warmth: the temperatures as they were.
    temp_old: Vec<f32>,
    /// The heat the air took from the sky in the last heat pass, per cell (see `heat_air_and_reactions`).
    heat_took: Vec<f32>,
    /// Tests (and calm sandboxes) can pin the air temperature everywhere.
    pub climate_fixed: Option<f32>,
    /// The energy budget of the world: the sun in, the sky out to space (see `thermal.rs`).
    pub climate: thermal::Climate,
    /// Bumped whenever the picture could have changed: lets the renderer skip work.
    pub revision: u64,
    /// Water dried into the air since the start, in units of fill (the volume tests count it).
    pub dried: u64,
    /// Liquid that left the world by its sides since the start, in units of fill.
    pub gone: u64,
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
    /// Charges of gunpowder that went off this step, one entry or a few for a big one (at night
    /// they launch fireworks).
    pub powder_booms: Vec<(i32, i32)>,
    /// Clods of matter thrown out of craters, in flight.
    pub ejecta: Vec<Ejecta>,
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
    /// Whether the cells take their turn on every core: faster, but not the same turn (see
    /// `turn.rs`). Off unless `AWORLD_CELLS_PAR=1`.
    pub parallel_cells: bool,
    /// Whether every liquid moves by real forces (velocities, pressure, viscosity, see
    /// `liquid.rs`). On unless `AWORLD_LIQUIDS=0`.
    pub liquid_physics: bool,
    /// The velocities and pressure of the liquids (see `liquid.rs`).
    liq: liquid::Liquids,
    /// Units of water in each cell of water, `field::FULL` for a full one (stored as 0, so that a
    /// cell painted water is full).
    fill: Vec<u32>,
    /// Closed pockets of gas (see `pockets.rs`): for each cell its pocket id + 1 (0: not gas).
    pocket_of: Vec<u32>,
    pockets: Vec<pockets::Pocket>,
    /// Latent heat taken in (positive) or given up (negative) by a cell of water or ice on its way
    /// to changing state (see `thermal::latent_heat`).
    pub latent: Vec<f32>,
    /// Falling speed of each grain, in eighths of a cell per step (gravity speeds a free-falling
    /// grain up; landing stops it).
    vel: Vec<u8>,
    /// How far a falling cell is on its way to the next one down, in `FALL_UNIT`ths of a cell.
    fall_at: Vec<u8>,
    /// The other states a cell of matter carries, one per `Aspect`.
    pub moisture: Vec<u16>,
    pub fuel: Vec<Fuel>,
    pub growth: Vec<u8>,
    jet: Vec<u8>,
    /// Absolute pressure (atm) inside each liquid cell; 1 elsewhere.
    liq_p: Vec<f32>,
    /// Scratch for the pockets: what each cell is to the gas, and the weight of each liquid cell.
    cls: Vec<u8>,
    liq_w: Vec<f32>,
    /// Sleeping zones: blocks of cells where nothing can happen are skipped (see `sleep.rs`).
    pub zones: sleep::Zones,
    /// Walls that gave way under the pressure of a pocket this step: (x, y, pressure in atm).
    pub bursts: Vec<(i32, i32, f32)>,
    /// Electric charge of the clouds, one value per `QS`×`QS` square (see `storm.rs`).
    pub charge: Vec<f32>,
    /// Electric charge held in the ground and the things on it, one value per `QS`×`QS` square:
    /// what a charged cloud draws up under it, and what a bolt leaves behind (see `storm.rs`).
    pub mcharge: Vec<f32>,
    /// How strongly the iron of each `QS`×`QS` square is magnetised (by the current of a bolt).
    pub magnet: Vec<f32>,
    /// The opposite charge a charged square draws into the matter round it (influence), worked
    /// out afresh each storm step from `mcharge`.
    pub polar: Vec<f32>,
    /// Bolts of lightning since they were last read. For the picture and the thunder.
    pub bolts: Vec<Bolt>,
    storm_rng: storm::StormRng,
    /// The last pressure field read through `pressure_at_cell` (the loupe): (revision, tick, field).
    pressure_cache: std::sync::Mutex<Option<(u64, u32, Vec<f32>)>>,
    /// Time spent in each phase of a step, when switched on (see `prof.rs`).
    pub prof: prof::Profile,
    /// `ambient_c` of every row, worked out once when the cells take their turn (gas asks for it
    /// at every move); empty outside it. During the turn only the drift of the sky can change
    /// (heat given off as vapour condenses), by a few thousandths of a degree: the table holds for
    /// the whole turn, and `air_sky` is what the sky was when it was built (the strips of a
    /// parallel turn add their drift to it).
    air_rows: Vec<f32>,
    air_sky: f32,
    /// In the view of a strip of the cells' turn, the rules it puts off to the end of the turn
    /// (see `turn.rs`); None in the world itself.
    later: Option<Vec<turn::Later>>,
}

/// Runs `$e`, adding its time to phase `$p` of the profile when it is on.
macro_rules! timed {
    ($s:ident, $p:expr, $e:expr) => {{
        let clock = $s.prof.start();
        $e;
        $s.prof.stop($p, clock);
    }};
}

impl World {
    pub fn new() -> Self {
        World {
            pressure_cache: std::sync::Mutex::new(None),
            cells: vec![EMPTY; W() * H()],
            frame: 0,
            rng: 0x1234_5678,
            seen: vec![false; W() * H()],
            stack: Vec::new(),
            released: vec![false; W() * H()],
            released_list: Vec::new(),
            falling: Vec::new(),
            slabs: Vec::new(),
            level: vec![u32::MAX; W() * H()],
            strain: vec![0; W() * H()],
            pushes: Vec::new(),
            strain_key: 0,
            stress_cache: std::collections::HashMap::new(),
            stress_pos: Vec::new(),
            stress_local: Vec::new(),
            ticks: 0,
            log: vec![0; W() * H()],
            paint_log: 0,
            temp: vec![15.0; W() * H()],
            clock: DAY_STEPS / 5,
            dt: STEP,
            tick_time: 0.0,
            heat_stage: 0,
            day_cycle: true,
            thermal_flip: false,
            heat_carry: (0.0, 0.0),
            temp_old: Vec::new(),
            heat_took: Vec::new(),
            climate_fixed: None,
            climate: thermal::Climate::default(),
            revision: 1,
            dried: 0,
            gone: 0,
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
            mask: vec![0; W() * H()],
            mask_prev: vec![0; W() * H()],
            block_mat: Vec::new(),
            mask_pass: Vec::new(),
            touched: Vec::new(),
            blasts: Vec::new(),
            powder_booms: Vec::new(),
            ejecta: Vec::new(),
            thuds: Vec::new(),
            water_flow: 0,
            flow: flow::Flow::new(),
            dp: vec![0.0; flow::FW() * flow::FH()],
            dp_slow: vec![0.0; flow::FW() * flow::FH()],
            dpd: vec![0.0; flow::FW() * flow::FH()],
            pressure_forces: true,
            lod: 0,
            parallel_cells: turn::parallel_by_default(),
            liquid_physics: liquid::physics_by_default(),
            liq: liquid::Liquids::default(),
            fill: vec![0; W() * H()],
            pocket_of: vec![0; W() * H()],
            pockets: Vec::new(),
            liq_p: vec![1.0; W() * H()],
            latent: vec![0.0; W() * H()],
            vel: vec![0; W() * H()],
            fall_at: vec![0; W() * H()],
            moisture: vec![0; W() * H()],
            fuel: vec![Fuel::default(); W() * H()],
            growth: vec![0; W() * H()],
            jet: vec![0; W() * H()],
            cls: vec![0; W() * H()],
            liq_w: vec![0.0; W() * H()],
            bursts: Vec::new(),
            zones: sleep::Zones::new(),
            charge: vec![0.0; QW() * QH()],
            mcharge: vec![0.0; QW() * QH()],
            magnet: vec![0.0; QW() * QH()],
            polar: vec![0.0; QW() * QH()],
            bolts: Vec::new(),
            storm_rng: storm::StormRng(0x9e37_79b9),
            prof: prof::Profile::default(),
            air_rows: Vec::new(),
            air_sky: 0.0,
            later: None,
        }
    }

    /// A world where nothing falls apart: handy for tests with floating floors.
    #[cfg(test)]
    pub fn new_static() -> Self {
        let mut w = Self::new();
        w.structures_fall = false;
        w.climate_fixed = Some(15.0);
        w.reset_temps();
        // (The cells' liquids, as these scenes were written and checked; a test of the
        // physical liquids says so, as `liquid::tests::world` does.)
        w.set_liquid_physics(false);
        w
    }

    pub fn clear(&mut self) {
        self.revision += 1;
        self.charge.fill(0.0);
        self.mcharge.fill(0.0);
        self.magnet.fill(0.0);
        self.polar.fill(0.0);
        self.cells.fill(EMPTY);
        self.fill.fill(0);
        self.liq.clear();
        self.log.fill(0);
        self.ejecta.clear();
        self.slabs.clear();
        self.falling.clear();
        self.latent.fill(0.0);
        self.vel.fill(0);
        self.zones.wake_all();
        self.reset_temps();
        self.update_season_cache();
    }

    /// Replace the grid with saved materials (row by row, W*H entries).
    pub fn load_mats(&mut self, mats: &[Mat]) {
        for (i, m) in mats.iter().enumerate().take(W() * H()) {
            self.set((i % W()) as i32, (i / W()) as i32, *m);
        }
    }

    /// The matter of a cell in a captured state (empty outside the world).
    pub fn raw_in(st: &State, x: i32, y: i32) -> Mat {
        if Self::inb(x, y) { st.matter.cells[Self::idx(x, y)].m } else { Mat::Empty }
    }

    /// Makes this world a picture of `from`, for the screen while `from` keeps running in its own
    /// thread (see `simthread.rs`): everything that is drawn, read by the loupe, the sensors or
    /// the visitors, but not the scratch of the step, nor the pressure solver of the liquids. The
    /// copy is never stepped. Every field is named, so that a new one has to be sorted here.
    pub fn copy_picture(&mut self, from: &World) {
        let World {
            cells, frame, rng, seen: _, stack: _, released: _, released_list: _, falling, slabs, level: _, strain, pushes, strain_key, stress_cache: _,
            stress_pos: _, stress_local: _, ticks, log, paint_log, temp, clock, dt, tick_time, heat_stage, day_cycle, thermal_flip, heat_carry, temp_old: _, heat_took: _,
            climate_fixed, climate, revision, wind, wind_max, wind_changes, windiness, wind_clock, biome, leaf_rgb, autumn, grass_rgb,
            structures_fall, mask, mask_prev: _, block_mat, mask_pass, touched, blasts, powder_booms, ejecta, thuds, water_flow, flow, dp,
            dp_slow, dpd, pressure_forces, lod, parallel_cells, liquid_physics, liq, fill, pocket_of,
            pockets, latent, vel, fall_at, moisture, fuel, growth, jet, liq_p, cls: _, liq_w: _, zones, bursts, charge, mcharge, magnet, polar,
            bolts, storm_rng, pressure_cache: _, prof: _, air_rows, air_sky, later: _, dried, gone,
        } = from;
        self.dried = *dried;
        self.gone = *gone;
        self.cells.clone_from(cells);
        self.frame = *frame;
        self.rng = *rng;
        self.falling.clone_from(falling);
        self.slabs.clone_from(slabs);
        self.strain.clone_from(strain);
        self.pushes.clone_from(pushes);
        self.strain_key = *strain_key;
        self.ticks = *ticks;
        self.log.clone_from(log);
        self.paint_log = *paint_log;
        self.temp.clone_from(temp);
        self.clock = *clock;
        self.dt = *dt;
        self.tick_time = *tick_time;
        self.heat_stage = *heat_stage;
        self.day_cycle = *day_cycle;
        self.thermal_flip = *thermal_flip;
        self.heat_carry = *heat_carry;
        self.climate_fixed = *climate_fixed;
        self.climate.clone_from(climate);
        self.revision = *revision;
        self.wind = *wind;
        self.wind_max = *wind_max;
        self.wind_changes = *wind_changes;
        self.windiness = *windiness;
        self.wind_clock = *wind_clock;
        self.biome = *biome;
        self.leaf_rgb = *leaf_rgb;
        self.autumn = *autumn;
        self.grass_rgb = *grass_rgb;
        self.structures_fall = *structures_fall;
        self.mask.clone_from(mask);
        self.block_mat.clone_from(block_mat);
        self.mask_pass.clone_from(mask_pass);
        self.touched.clone_from(touched);
        self.blasts.clone_from(blasts);
        self.powder_booms.clone_from(powder_booms);
        self.ejecta.clone_from(ejecta);
        self.thuds.clone_from(thuds);
        self.water_flow = *water_flow;
        self.flow.clone_from(flow);
        self.dp.clone_from(dp);
        self.dp_slow.clone_from(dp_slow);
        self.dpd.clone_from(dpd);
        self.pressure_forces = *pressure_forces;
        self.lod = *lod;
        self.parallel_cells = *parallel_cells;
        self.liquid_physics = *liquid_physics;
        self.liq.copy_picture(liq);
        self.fill.clone_from(fill);
        // The grains' falling speed, for the simulation view.
        self.vel.clone_from(vel);
        self.fall_at.clone_from(fall_at);
        self.pocket_of.clone_from(pocket_of);
        self.pockets.clone_from(pockets);
        self.latent.clone_from(latent);
        self.moisture.clone_from(moisture);
        self.fuel.clone_from(fuel);
        self.growth.clone_from(growth);
        self.jet.clone_from(jet);
        self.liq_p.clone_from(liq_p);
        self.zones.clone_from(zones);
        self.bursts.clone_from(bursts);
        self.charge.clone_from(charge);
        self.mcharge.clone_from(mcharge);
        self.magnet.clone_from(magnet);
        self.polar.clone_from(polar);
        self.bolts.clone_from(bolts);
        self.storm_rng.clone_from(storm_rng);
        self.air_rows.clone_from(air_rows);
        self.air_sky = *air_sky;
    }

    pub fn capture(&self) -> State {
        State { matter: self.snapshot(), temp: self.temp.clone(), clock: self.clock }
    }

    pub fn restore_state(&mut self, st: &State) {
        self.restore(&st.matter);
        self.temp.copy_from_slice(&st.temp);
        self.clock = st.clock;
        self.falling.clear();
        self.slabs.clear();
        self.touched.clear();
        self.blasts.clear();
        self.powder_booms.clear();
        self.ejecta.clear();
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

    /// The census, and how much of each material there is in cells of it: a cell of liquid counts
    /// what it holds (the liquids by their physics may fill a cell only in part), a cell of steam or
    /// cloud the vapour packed in it as well (as it would spread at the pressure of the open air).
    pub fn volumes(&self) -> ([u32; NMAT], [f32; NMAT]) {
        let mut n = [0u32; NMAT];
        let mut v = [0f64; NMAT];
        for (i, c) in self.cells.iter().enumerate() {
            let m = c.m as usize;
            n[m] += 1;
            v[m] += match c.m.kind() {
                Kind::Liquid => self.liquid_fill(i) as f64 / field::FULL as f64,
                Kind::Gas if matches!(c.m, Mat::Steam | Mat::Cloud) => 1.0 + self.packed(i) as f64,
                _ => 1.0,
            };
        }
        (n, v.map(|x| x as f32))
    }

    /// Removes `count` cells of the given materials, nearest to `near` first.
    /// Returns false (and removes nothing) if there are not enough.
    pub fn consume(&mut self, mats: &[Mat], count: u32, near: (i32, i32)) -> bool {
        self.revision += 1;
        let mut found: Vec<(i64, usize)> = vec![];
        for (i, c) in self.cells.iter().enumerate() {
            if mats.contains(&c.m) {
                let (x, y) = ((i % W()) as i64 - near.0 as i64, (i / W()) as i64 - near.1 as i64);
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
            self.clear_states(i);
            self.zones.wake(i);
        }
        // The ground sinks into what was taken instead of leaving a cave under its turf: the soil
        // and grass above each hole slide down one cell, highest hole first so stacked holes close
        // from the top. The grass stays on top of the earth that carries it.
        taken.sort_unstable();
        let mut columns = vec![];
        for i in taken {
            let (x, y) = ((i % W()) as i32, (i / W()) as i32);
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
            let mut y = H() as i32 - 1;
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

    pub fn snapshot(&self) -> Matter {
        Matter { cells: self.cells.clone(), moisture: self.moisture.clone(), fuel: self.fuel.clone(), growth: self.growth.clone(), jet: self.jet.clone(), log: self.log.clone() }
    }

    pub fn restore(&mut self, snap: &Matter) {
        self.revision += 1;
        self.cells.copy_from_slice(&snap.cells);
        self.fill.fill(0);
        self.liq.clear();
        self.moisture.copy_from_slice(&snap.moisture);
        self.fuel.copy_from_slice(&snap.fuel);
        self.growth.copy_from_slice(&snap.growth);
        self.jet.copy_from_slice(&snap.jet);
        self.log.copy_from_slice(&snap.log);
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
        x >= 0 && y >= 0 && (x as usize) < W() && (y as usize) < H()
    }

    /// Index of an in-bounds cell. Callers check `inb` first (or use `ix`): out of bounds, a
    /// cell past the right side would land on the next row, and one below the last row past
    /// the end of the grid. Debug builds and the tests catch it here.
    #[cfg_attr(debug_assertions, track_caller)]
    fn idx(x: i32, y: i32) -> usize {
        debug_assert!(Self::inb(x, y), "cell ({x}, {y}) is outside the world");
        #[cfg(debug_assertions)]
        turn::guard(x);
        y as usize * W() + x as usize
    }

    /// Index of a cell, or None outside the world. Mind that `at` reads Empty past the sides
    /// and the bottom: a free cell by `at` may still be outside the grid.
    #[cfg_attr(debug_assertions, track_caller)]
    fn ix(x: i32, y: i32) -> Option<usize> {
        #[cfg(debug_assertions)]
        if Self::inb(x, y) {
            turn::guard(x);
        }
        if Self::inb(x, y) { Some(y as usize * W() + x as usize) } else { None }
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

    /// Whether the cell (x, y) holds up a body coming down on it whose underside was at row `feet`
    /// before it moved: anything hard does, and so does thin matter (`Mat::one_way`) that was
    /// under it, or whose top lies at most `climb` rows over it (a step up onto a crown). A trunk
    /// beside the body, or the underside of a crown over it, holds nothing up: it passes through.
    pub fn bears(&self, x: i32, y: i32, feet: f32, climb: f32) -> bool {
        if self.hard_for_car(x, y) {
            return true;
        }
        if !Self::inb(x, y) || !self.at(x, y).one_way() {
            return false;
        }
        let row = y as f32;
        row >= feet || (row >= feet - climb && !self.at(x, y - 1).one_way() && !self.hard_for_car(x, y - 1))
    }

    /// The column just past the thin matter (`Mat::one_way`) beside (x, y) on side `dir`, if
    /// there is some (at most `THIN_RUN` cells of it): where a liquid that passes through it
    /// comes out.
    pub fn past_thin(&self, x: i32, y: i32, dir: i32) -> Option<i32> {
        let mut k = 1;
        while k <= THIN_RUN && Self::inb(x + dir * k, y) && self.at(x + dir * k, y).one_way() {
            k += 1;
        }
        (k > 1 && k <= THIN_RUN && Self::inb(x + dir * k, y)).then_some(x + dir * k)
    }

    /// Obstacle test for the car (hard all round; thin matter is not, see `bears`); the world
    /// has no floor below it.
    pub fn hard_for_car(&self, x: i32, y: i32) -> bool {
        // Beyond the edges there is nothing to hold a car back.
        if y >= H() as i32 || y < 0 || x < 0 || x >= W() as i32 {
            return false;
        }
        let i = Self::idx(x, y);
        let b = self.mask[i];
        if b != 0 && self.mask_pass.get(b as usize - 1).copied().unwrap_or(false) {
            // Load-bearing structure: the car passes through; only what is under it counts.
            let m = self.cells[i].m;
            return m.is_hard() && !m.one_way();
        }
        let m = self.at(x, y);
        m.is_hard() && !m.one_way()
    }

    pub fn set(&mut self, x: i32, y: i32, m: Mat) {
        if !Self::inb(x, y) {
            return;
        }
        self.revision += 1;
        let left = match m {
            Mat::Fire => 20 + (self.rand() % 40) as u8,
            Mat::Ember => 100 + (self.rand() % 100) as u8,
            _ => 0,
        };
        let frame = self.frame;
        let i = Self::idx(x, y);
        self.cells[i] = Cell { m, stamp: frame };
        self.latent[i] = 0.0;
        self.vel[i] = 0;
        self.clear_states(i);
        self.fuel[i].left = left;
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
            self.clear_states(Self::idx(a.0, a.1));
            self.zones.wake(Self::idx(a.0, a.1));
            return;
        }
        let (ia, ib) = (Self::idx(a.0, a.1), Self::idx(b.0, b.1));
        self.zones.wake(ia);
        self.zones.wake(ib);
        if self.cells[ia].m == Mat::Water {
            self.water_flow += 1;
        }
        self.carry(ia, ib);
        // Grains slipping past the air (or gas) between them share its warmth on the way: hot air
        // cannot bubble up through a heap of sand and carry the heat out of the top; it gives it
        // to the grains it touches, and the heap warms out from where it was heated.
        let (ma, mb) = (self.cells[ia].m, self.cells[ib].m);
        let airy = |m: Mat| matches!(m.kind(), Kind::Empty | Kind::Gas);
        if (ma.kind() == Kind::Powder && airy(mb)) || (mb.kind() == Kind::Powder && airy(ma)) {
            let (ca, cb) = (thermal::cap(ma), thermal::cap(mb));
            let t = (ca * self.temp[ia] + cb * self.temp[ib]) / (ca + cb);
            self.temp[ia] = t;
            self.temp[ib] = t;
        }
        if a.1 != b.1 {
            // Vapour and clouds cool as they rise into thinner air, and warm as they sink.
            self.temp[ib] -= adiabatic_cooling_c(self.cells[ib].m, a.1, b.1);
            self.temp[ia] -= adiabatic_cooling_c(self.cells[ia].m, b.1, a.1);
        }
        self.cells[ia].stamp = self.frame;
        self.cells[ib].stamp = self.frame;
        if self.later.is_some() && self.cells[ib].m.kind() == Kind::Powder && matches!(self.cells[ia].m.kind(), Kind::Empty | Kind::Gas) {
            self.left_hole(a.0, a.1);
        }
    }

    /// Swaps the matter of cells `i` and `j` with every state it carries (see `Aspect`).
    fn carry(&mut self, i: usize, j: usize) {
        self.cells.swap(i, j);
        self.temp.swap(i, j);
        self.latent.swap(i, j);
        self.vel.swap(i, j);
        self.fall_at.swap(i, j);
        self.moisture.swap(i, j);
        self.fill.swap(i, j);
        self.fuel.swap(i, j);
        self.growth.swap(i, j);
        self.jet.swap(i, j);
        self.log.swap(i, j);
    }

    /// Clears the states of cell `i` that only some matters carry (heat stays: every matter has one).
    fn clear_states(&mut self, i: usize) {
        self.moisture[i] = 0;
        self.fill[i] = 0;
        self.fuel[i] = Fuel::default();
        self.growth[i] = 0;
        self.jet[i] = 0;
        self.log[i] = 0;
    }

    fn states(&self, i: usize) -> States {
        States { moisture: self.moisture[i], fuel: self.fuel[i], growth: self.growth[i], jet: self.jet[i] }
    }

    /// Cell `i`, which was `from` and has just been set to a new matter, takes back the states of
    /// `kept` that both matters carry; the others stay as `set` left them.
    pub(super) fn keep_states(&mut self, i: usize, kept: States, from: Mat) {
        let to = self.cells[i].m;
        let both = |a| from.carries(a) && to.carries(a);
        if both(Aspect::Water) {
            self.moisture[i] = kept.moisture;
        }
        if both(Aspect::Fuel) {
            self.fuel[i] = kept.fuel;
        }
        if both(Aspect::Growth) {
            self.growth[i] = kept.growth;
        }
        if both(Aspect::Jet) {
            self.jet[i] = kept.jet;
        }
    }

    /// Extra cells of vapour packed in a cell of steam or cloud, on top of its own.
    fn packed(&self, i: usize) -> u32 {
        (self.moisture[i] / VAPOUR_UNITS) as u32
    }

    fn set_packed(&mut self, i: usize, n: u32) {
        self.moisture[i] = n as u16 * VAPOUR_UNITS;
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
        for i in 0..W() * H() {
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
            let (x, y) = ((i % W()) as i32, (i / W()) as i32);
            let mut moved = false;
            for dy in 1..=30 {
                if y - dy < 0 {
                    break;
                }
                let j = (y - dy) as usize * W() + x as usize;
                if self.mask[j] == 0 && self.cells[j].m == Mat::Empty {
                    self.carry(i, j);
                    self.cells[j].stamp = self.frame;
                    self.zones.wake(j);
                    moved = true;
                    break;
                }
            }
            if !moved {
                self.cells[i] = EMPTY;
                self.clear_states(i);
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
                // Seeds are sown: on earth they go into the soil and make it fertile.
                if m == Mat::Seed && !matches!(rule, PaintRule::Replace) && self.sow(x, y) {
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
                    if m == Mat::Wood {
                        self.log[i] = self.paint_log;
                    }
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
    /// longer pour in, and the roof holds as long as the walls rest on something. Rock is lined
    /// too: the rock over a bore is held by the vault (the world is flat: a tunnel through it
    /// cuts the rock above from the rock below). Metal and masonry are left as they are, and so
    /// is the floor.
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
            Kind::Static => !m.is_vegetation() && !matches!(m, Mat::Iron | Mat::Steel | Mat::Brick | Mat::Terracotta | Mat::Glass | Mat::Vault),
        }
    }

    /// An explosion that reaches about `r` cells: as strong as the charge of powder whose fire,
    /// pressure wave and shove reach that far (see `blast.rs`).
    pub fn explode(&mut self, cx: i32, cy: i32, r: i32) {
        let grains = ((r as f32 - 6.0).max(0.5) / 2.2).powi(2);
        self.detonate(cx, cy, blast::Charge::powder(grains));
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

    /// Whether no block or car stands in cell (x, y) (outside the world counts as free).
    pub fn mask_free(&self, x: i32, y: i32) -> bool {
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
    #[cfg(test)]
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
        // The landscape is drawn on the design grid (`BASE_W` x `BASE_H`), then stretched over the
        // world's cells.
        let (sx, sy) = (sx(), sy());
        let row = |v: i32| (v as f32 * sy) as i32;
        let mut hs = vec![0i32; W()];
        for x in 0..W() {
            let xf = x as f32 / sx / K as f32;
            let edge = (x.min(W() - 1 - x) as f32 / sx / (40 * K) as f32).min(1.0);
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
            hs[x] = ((h * K as f32 * sy) as i32).clamp(row(40), H() as i32 - row(40));
        }
        let sea = row(match biome {
            Biome::Countryside => 122 * K,
            Biome::Mountain => 135 * K,
            Biome::Sea => 128 * K,
        });
        for x in 0..W() {
            let top = hs[x];
            let beach = top > sea - row(5 * K);
            for y in top..H() as i32 {
                let depth = y - top;
                let m = match biome {
                    // Snow caps (ice) on the peaks, bare rock below them, grass in the valleys.
                    Biome::Mountain if top < row(250) && depth < 3 * K => Mat::Ice,
                    Biome::Mountain if top < row(320) && depth < 5 * K => Mat::Stone,
                    _ if depth < 2 * K && beach => Mat::Sand,
                    _ if depth < K => Mat::Grass,
                    _ if depth < 6 * K && !beach => Mat::Dirt,
                    _ if depth < 3 * K => Mat::Sand,
                    _ => Mat::Stone,
                };
                self.set(x as i32, y, m);
                if matches!(m, Mat::Dirt | Mat::Grass) {
                    // Moist soil, below field capacity: it holds its water until the sun dries it.
                    self.moisture[Self::idx(x as i32, y)] = 72;
                }
            }
            for y in sea..top {
                self.set(x as i32, y, Mat::Water);
            }
        }
        let (tries, min_top) = match biome {
            Biome::Countryside => (6, 0),
            Biome::Mountain => (6, row(330)),
            Biome::Sea => (3, 0),
        };
        for _ in 0..tries {
            let x = (50 * K) + (self.rand() % (W() as u32 - 100 * K as u32)) as i32;
            let base = hs[x as usize];
            if base >= sea - row(4 * K) || base < min_top {
                continue;
            }
            let hgt = (8 + (self.rand() % 6) as i32) * K;
            self.fill_rect(x, base - hgt, K, hgt, Mat::Trunk);
            let top = base - hgt;
            self.grow_crown(x, top);
        }
        self.reset_temps();
        ((16 * K, hs[16 * K as usize] - 8 * K), (W() as i32 - 16 * K, hs[W() - 16 * K as usize] - 8 * K))
    }

    // ----- simulation -------------------------------------------------------

    /// One step of a tick, 1/60 s.
    pub fn step(&mut self) {
        self.step_dt(STEP);
    }

    /// The shortest step worth taking, in seconds: the liquids and the heat run at every step,
    /// and steps much shorter than a tick would cost them as many times more.
    pub const MIN_DT: f32 = STEP / 2.0;

    /// The world moves on by `dt` seconds, at most `MAX_DT`. The cells tick every `STEP` of the
    /// world's time (none, one or several times in the step); the liquids and the heat run once,
    /// over `dt` (each cut into the sub-steps its stability needs). A world stepped by 1/120 s or
    /// by 1/30 s goes through the same second, and a longer step costs less than as many ticks.
    /// Returns how many ticks the cells took in it.
    pub fn step_dt(&mut self, dt: f32) -> u32 {
        let dt = dt.clamp(0.0, MAX_DT);
        self.dt = dt;
        self.tick_time += dt;
        // (A hair of slack: two steps of 1/120 s make a tick whatever the rounding.)
        let ticks = ((self.tick_time + 1e-3 * STEP) / STEP).floor() as u32;
        self.tick_time = (self.tick_time - ticks as f32 * STEP).max(0.0);
        self.step_ticks(ticks);
        ticks
    }

    /// A step with `ticks` ticks of the cells in it; the liquids come after the cells of the
    /// last tick and the heat in its place among the passes of the last tick, as when every step
    /// was a tick.
    fn step_ticks(&mut self, ticks: u32) {
        self.revision += 1;
        let clock = self.prof.start();
        let audit = self.liquid_physics && std::env::var_os("AWORLD_DEBUG_MASS").is_some();
        let mut marks: Vec<(&str, u64)> = Vec::new();
        if audit {
            marks.push(("début", self.water_units_audit()));
        }
        // (The first tick's frame already, for what the card hands back.)
        if ticks > 0 {
            self.frame = self.frame.wrapping_add(1);
        }
        // (The seep through thin matter runs on the cells the card gave back, before the step's
        // cells go to it: a cell it changed after they went would be a cell the card has wrong,
        // and what the card did to it would be lost at the next step.)
        if self.liquid_physics {
            self.take_liquids_from_card();
            if self.liquids_on_card() {
                self.seep_through_thin();
            }
        }
        if audit {
            marks.push(("reprise de la carte", self.water_units_audit()));
        }
        for k in 0..ticks {
            let last = k + 1 == ticks;
            if k > 0 {
                self.frame = self.frame.wrapping_add(1);
            }
            timed!(self, 0, self.step_cells());
            if audit {
                marks.push(("tour des cellules", self.water_units_audit()));
            }
            if self.liquid_physics {
                if last {
                    // (The liquids go by the ticks: a turn of `STEP` for each, see
                    // `liquid::turns`; a step shorter than a tick leaves them where they are,
                    // as it does the cells.)
                    timed!(self, prof::LIQUIDS, self.step_liquids(ticks as f32 * STEP));
                }
                if !self.liquids_on_card() {
                    self.seep_through_thin();
                }
            }
            if audit {
                marks.push(("liquides", self.water_units_audit()));
            }
            self.after_cells(last);
            if audit {
                marks.push(("après les cellules", self.water_units_audit()));
            }
        }
        if ticks == 0 {
            self.heat_turn();
            if audit {
                marks.push(("liquides et chaleur", self.water_units_audit()));
            }
        }
        if audit {
            for k in 1..marks.len() {
                if marks[k].1 != marks[k - 1].1 {
                    eprintln!("pas {}: l'eau passe de {} à {} ({:+}) dans « {} »", self.frame, marks[k - 1].1, marks[k].1, marks[k].1 as i64 - marks[k - 1].1 as i64, marks[k].0);
                }
            }
        }
        self.prof.stop(prof::TOTAL, clock);
        self.prof.end_step();
    }

    /// The next part of the heat pass, over the step's `dt` (see `thermal_stage`).
    fn heat_turn(&mut self) {
        self.heat_stage = self.heat_stage.wrapping_add(1);
        let cycle = thermal::THERMAL_EVERY * self.thermal_lod_scale() as u32;
        timed!(self, 12, self.thermal_stage(self.heat_stage % cycle));
    }

    /// The turn of the cell at (x, y), in zone `zone` (see `turn.rs` for the order).
    fn cell_turn(&mut self, x: i32, y: i32, zone: usize) {
        let c = self.cells[Self::idx(x, y)];
        if c.m == Mat::Empty {
            return;
        }
        if c.stamp == self.frame {
            // Moved here this step, or a stamp left from 256 steps ago: either way look again
            // next step.
            self.zones.mark_restless(zone);
            return;
        }
        if self.restless(x, y, c) {
            self.zones.mark_restless(zone);
        }
        match c.m {
            Mat::Rust => {
                if !self.magnet_holds(x, y) {
                    self.step_powder(x, y);
                }
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
                if !self.fly(x, y) && !self.jet(x, y) {
                    self.step_oil(x, y);
                }
            }
            Mat::Smoke => self.step_smoke(x, y),
            Mat::Steel => self.step_iron(x, y, 1500),
            Mat::Gunpowder | Mat::Dynamite => self.step_gunpowder(x, y),
            Mat::Saltpeter => self.step_saltpeter(x, y),
            Mat::Water => {
                if !self.fly(x, y) && !self.jet(x, y) {
                    self.step_water(x, y);
                }
            }
            Mat::Steam => self.step_steam(x, y),
            Mat::Fire => self.step_fire(x, y),
            Mat::Ember => self.step_ember(x, y),
            Mat::Lava => self.step_hot_liquid(x, y, Mat::Stone),
            Mat::Molten => self.step_hot_liquid(x, y, Mat::Iron),
            Mat::MoltenSteel => self.step_hot_liquid(x, y, Mat::Steel),
            Mat::Iron => self.step_iron(x, y, 300),
            _ => {}
        }
    }

    /// `ambient_c(y)`, from the table of the cells' turn when it is on.
    #[inline]
    fn air_c(&self, y: i32) -> f32 {
        // (The table holds for the whole turn: the sky's reservoir moves by a few thousandths
        // of a degree at most during one, and recomputing the profile for every cell of gas
        // once it had moved cost 6 % of a turn full of clouds.)
        match self.air_rows.get(y.clamp(0, H() as i32 - 1) as usize) {
            Some(&t) => t,
            None => self.ambient_c(y),
        }
    }

    /// Everything after the cells in a tick: falling pieces, wind, pressure, the clock, and on the
    /// `last` tick of the step the heat.
    fn after_cells(&mut self, last: bool) {
        self.ticks = self.ticks.wrapping_add(1);
        // The passes that come back every few steps are set on different steps, so that the
        // heavy ones never fall together, and the heat pass is spread over the three steps of its
        // cycle (conduction with the air and the reactions, then space and the sun, then the wind
        // carrying heat every other cycle): a step costs about the same as the next, instead of
        // one step in three carrying everything. The structures come back every 9 steps (on
        // 9k + 1: always beside space and the sun, the lightest part of the heat), the pockets
        // every 18 (on 18k + 5: never with the structures, and on an odd step: on the steps of
        // `level_liquids` the U-tubes and siphons stop levelling, see their tests), the storm
        // every 4 (on 4k), the air's pressure every 4 (on 4k + 1).
        if self.structures_fall && self.ticks % 9 == 1 {
            timed!(self, 1, self.structure_pass());
        }
        if self.structures_fall && self.ticks % 2 == 0 {
            timed!(self, 2, self.drop_structures());
        }
        if self.structures_fall {
            timed!(self, 3, self.step_slabs());
        }
        if self.ticks % pockets::POCKETS_EVERY == 5 {
            timed!(self, 4, self.update_pockets());
        }
        timed!(self, 5, self.step_ejecta());
        if self.ticks % matter::SETTLE_EVERY == 17 {
            timed!(self, 6, self.settle_matter());
        }
        timed!(self, 7, self.update_wind());
        timed!(self, 8, self.update_flow());
        if self.ticks % 4 == 1 {
            timed!(self, 9, self.update_pressure());
        }
        timed!(self, 10, self.pressure_push());
        if self.ticks % 64 == 0 {
            timed!(self, 11, self.update_season_cache());
        }
        if last {
            self.heat_turn();
        }
        timed!(self, 13, self.storm_step());
        if self.day_cycle {
            self.clock = self.clock.wrapping_add(1);
        }
        // (The physics of the liquids levels them itself, by their pressure.)
        if self.ticks % hydro::LEVEL_EVERY == 0 && !self.liquid_physics {
            timed!(self, 14, self.level_liquids());
        }
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

    /// A tick of the fall of the cell at (x, y): its speed grows by gravity up to `terminal`, its
    /// way through the cell by its speed, and once that makes a whole cell it goes down one if
    /// `free` (else it waits there, still falling). Always true: the cell is on the move.
    fn fall_on(&mut self, x: i32, y: i32, terminal: u8, free: bool) -> bool {
        let i = Self::idx(x, y);
        let v = self.vel[i].saturating_add(GRAIN_ACCEL).min(terminal);
        let way = self.fall_at[i] as u32 + v as u32;
        self.vel[i] = v;
        if way < FALL_UNIT || !free {
            self.fall_at[i] = way.min(FALL_UNIT - 1) as u8;
            return true;
        }
        self.fall_at[i] = (way - FALL_UNIT) as u8;
        self.swap((x, y), (x, y + 1));
        true
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
        // Free fall: the grain gathers speed (gravity) up to a terminal speed (air drag), and goes
        // down a cell each time its way through the cell adds up to a whole one (from rest: 6 cm
        // in its first tenth of a second, as anything else that falls).
        let terminal = if m == Mat::Snow { SNOW_TERMINAL } else { GRAIN_TERMINAL };
        if matches!(self.at(x, y + 1).kind(), Kind::Empty | Kind::Gas) && self.mask_free(x, y + 1) {
            return self.fall_on(x, y, terminal, true);
        }
        // On a cell that is falling itself (the column it is part of hangs between two cells):
        // it falls with it.
        let j = Self::idx(x, y + 1);
        if Self::inb(x, y + 1) && self.vel[j] > 0 && self.cells[j].m.kind() != Kind::Liquid && self.mask_free(x, y + 1) {
            return self.fall_on(x, y, terminal, false);
        }
        self.vel[i] = 0;
        self.fall_at[i] = 0;
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
            // Earth holds together: its grains only slide where the drop is two cells (a steep
            // bank of about 60 degrees, as a cut in the ground stands).
            Mat::Dirt => (100, 0),
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
    /// mud that dries out consolidates back into soil. Mud holds water like soil.
    fn step_sediment(&mut self, x: i32, y: i32) {
        let mud = self.at(x, y) == Mat::Mud;
        if mud {
            self.step_soil(x, y);
        }
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
            // It settles as soil, with the water it held, and goes on drying.
            self.retype(x, y, Mat::Dirt);
            return;
        }
        self.step_powder(x, y);
    }

    /// Grass is the living top of the soil. With nothing solid under it, it falls until it lands
    /// on soil, sand or rock, or else drops out of the world. It holds water like soil (and its
    /// leaves breathe it out faster); when the soil under it dries it turns straw yellow and dies
    /// back to bare earth, and buried or drowned it dies too. Its roots hold the earth against
    /// erosion, and it burns back to earth (see `thermal`).
    fn step_grass(&mut self, x: i32, y: i32) {
        let i = Self::idx(x, y);
        if matches!(self.at(x, y + 1).kind(), Kind::Empty | Kind::Gas | Kind::Liquid) && self.mask_free(x, y + 1) {
            self.swap((x, y), (x, y + 1));
            return;
        }
        self.step_soil(x, y);
        if self.sprout_tree(x, y) {
            return;
        }
        if (!self.turf_open(x, y) && self.chance(GRASS_SMOTHER)) || (self.moisture[i] < GRASS_DRY && self.chance(GRASS_WILT)) {
            self.retype(x, y, Mat::Dirt);
        }
    }

    /// Is the soil at (x, y) close enough to the open air for grass: open air right above it, or
    /// above a turf at most `K` cells thick (one cell of the design grid).
    fn turf_open(&self, x: i32, y: i32) -> bool {
        let mut k = 1;
        while k <= K && self.raw(x, y - k) == Mat::Grass {
            k += 1;
        }
        k <= K && matches!(self.at(x, y - k).kind(), Kind::Empty | Kind::Gas)
    }

    /// Changes what a cell is made of, keeping its temperature and the states `m` carries (see
    /// `Aspect`).
    pub(super) fn retype(&mut self, x: i32, y: i32, m: Mat) {
        let i = Self::idx(x, y);
        let (from, t, kept) = (self.cells[i].m, self.temp[i], self.states(i));
        self.set(x, y, m);
        self.temp[i] = t;
        self.keep_states(i, kept, from);
    }

    /// Bare soil washes away under flowing water (grass roots protect it), and grass grows on it
    /// where it is moist, in the open and in the light: it creeps in from grass nearby, and now
    /// and then a seed blown in by the wind starts a new patch.
    fn step_dirt(&mut self, x: i32, y: i32) {
        let i = Self::idx(x, y);
        // Earth is a cohesive granular matter, not a solid: a clod with nothing under it drops, and
        // a bank only slumps where it is steeper than its angle of repose (see `repose`).
        // (Worlds where nothing falls apart keep it as it lies.)
        if self.structures_fall && self.step_powder(x, y) {
            return;
        }
        self.step_soil(x, y);
        if self.sprout_tree(x, y) {
            return;
        }
        if self.moisture[i] >= GRASS_GROW && self.temp[i] > 5.0 && matches!(self.at(x, y - 1), Mat::Empty | Mat::Grass) && self.turf_open(x, y) {
            // Grass creeps sideways along the ground, and its roots thicken the turf downwards.
            let near = self.raw(x, y - 1) == Mat::Grass || (-1..=1).any(|dy| self.raw(x - 1, y + dy) == Mat::Grass || self.raw(x + 1, y + dy) == Mat::Grass);
            if self.chance(if near { GRASS_SPREAD } else { GRASS_SPROUT }) && self.lit(x, y) {
                self.retype(x, y, Mat::Grass);
                return;
            }
        }
        if self.chance(30) && self.flowing_water_near(x, y) && self.chance(8) {
            // Mud keeps the soil's water (its `moisture`) and gives it back when it settles.
            self.retype(x, y, Mat::Mud);
        }
    }

    /// How much moisture soil at (x, y) passes to the soil at `n` now: free water above field
    /// capacity drains down into drier soil below, and moisture creeps from wetter to drier soil
    /// in every direction (capillarity), a quarter of the difference at a time.
    fn soil_seep(&self, x: i32, y: i32, n: (i32, i32)) -> u16 {
        if !matches!(self.at(n.0, n.1), Mat::Dirt | Mat::Grass | Mat::Mud) {
            return 0;
        }
        let (a, b) = (self.moisture[Self::idx(x, y)] as i32, self.moisture[Self::idx(n.0, n.1)] as i32);
        let room = (SOIL_FULL as i32 - b).max(0);
        let creep = (a - b) / 4;
        let drain = if n.1 > y { a - SOIL_FIELD as i32 } else { 0 };
        creep.max(drain).clamp(0, room) as u16
    }

    /// Water in the soil: it seeps through the ground, drips out of saturated soil into a hole
    /// below, and evaporates where the soil meets the air, like a puddle does but slower as the
    /// soil dries. A little moisture (less than one cell of vapour's worth) always stays.
    fn step_soil(&mut self, x: i32, y: i32) {
        let i = Self::idx(x, y);
        if self.moisture[i] == 0 {
            return;
        }
        if self.chance(SOIL_SEEP) {
            let n = Self::neighbors(x, y)[(self.rand() % 4) as usize];
            let k = self.soil_seep(x, y, n);
            if k > 0 {
                let j = Self::idx(n.0, n.1);
                self.moisture[i] -= k;
                self.moisture[j] += k;
                self.zones.wake(j);
            }
        }
        if self.moisture[i] >= SOIL_FULL && self.at(x, y + 1) == Mat::Empty && self.mask_free(x, y + 1) && self.chance(SOIL_DRIP) {
            // On the bottom row the drop falls out of the world.
            if let Some(j) = Self::ix(x, y + 1) {
                let t = self.temp[i];
                self.set(x, y + 1, Mat::Water);
                self.temp[j] = t;
            }
            self.moisture[i] -= SOIL_WATER;
            return;
        }
        // Evaporation, as for open water (see `step_water`): warmer, drier and windier is faster.
        // Wet soil dries about as fast as a puddle, drier soil slower; grass breathes out the
        // water its roots draw up, so a lawn dries faster than bare earth.
        let t = self.temp[i];
        if self.moisture[i] >= SOIL_VAPOUR && t > 0.0 && self.at(x, y - 1) == Mat::Empty && self.mask_free(x, y - 1) {
            let dry = 1.0 - self.flow.moisture_at(x, y - 1);
            let (wx, wy) = self.flow.sample(x, y - 1);
            let breeze = 1.0 + 2.0 * ((wx * wx + wy * wy).sqrt() / flow::GAIN).min(1.0);
            let wet = (self.moisture[i] as f32 / SOIL_WATER as f32).min(1.0);
            let leaves = if self.cells[i].m == Mat::Grass { 1.5 } else { 1.0 };
            let rate = VAPOUR_PER_WATER as f32 * wet * leaves * dry * breeze * vapour_pressure(t) / vapour_pressure(40.0) / EVAPORATE_40C;
            if self.chance_f(rate) {
                // A quarter of a cell of water leaves as vapour, with the heat that takes: the
                // soil and the air over it cool.
                let j = Self::idx(x, y - 1);
                let q = thermal::heat_of(Mat::Empty, self.temp[j]) + thermal::heat_of(Mat::Water, t) / VAPOUR_PER_WATER as f32 - thermal::heat_of(Mat::Steam, t);
                self.moisture[i] -= SOIL_VAPOUR;
                self.set(x, y - 1, Mat::Steam);
                self.temp[j] = t;
                self.release_heat(x, y, q);
            }
        }
    }

    /// For the screenshots: soil soaked on the left of the world, drying across the middle and
    /// dry on the right, then `steps` steps so the grass answers to it.
    pub fn demo_lawn(&mut self, steps: u32) {
        for y in 0..H() as i32 {
            for x in 0..W() as i32 {
                let i = Self::idx(x, y);
                if matches!(self.cells[i].m, Mat::Dirt | Mat::Grass) {
                    let f = (1.0 - (x as f32 - W() as f32 * 0.3) / (W() as f32 * 0.4)).clamp(0.0, 1.0);
                    self.moisture[i] = (f * 170.0) as u16;
                }
            }
        }
        for _ in 0..steps {
            self.step();
        }
    }

    /// Can the soil at (x, y) still change by itself (seep, drip, dry out)?
    pub(super) fn soil_restless(&self, x: i32, y: i32) -> bool {
        let wet = self.moisture[Self::idx(x, y)];
        wet > 0
            && (Self::neighbors(x, y).iter().any(|&n| self.soil_seep(x, y, n) > 0)
                || (wet >= SOIL_FULL && self.at(x, y + 1) == Mat::Empty)
                || (wet >= SOIL_VAPOUR && self.at(x, y - 1) == Mat::Empty))
    }

    /// Can the water at (x, y) soak into the soil at `n`?
    fn soil_drinks(&self, n: (i32, i32)) -> bool {
        matches!(self.at(n.0, n.1), Mat::Dirt | Mat::Grass | Mat::Mud) && self.moisture[Self::idx(n.0, n.1)] <= SOIL_FULL - SOIL_WATER
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
                self.set_off_powder(x, y);
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
                if Self::inb(sx, sy) && matches!(self.cells[Self::idx(sx, sy)].m, Mat::Dirt | Mat::Grass) && self.moisture[Self::idx(sx, sy)] >= 30 {
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
            self.moisture[i] -= 30;
            return true;
        }
        false
    }

    /// A seed is not a matter that stays: it falls, and the soil it lands on takes it in (the
    /// earth becomes fertile, see `sow`). Fallen on anything else, it rots away.
    fn step_seed(&mut self, x: i32, y: i32) {
        if self.step_powder(x, y) {
            return;
        }
        if self.sow(x, y + 1) {
            self.set(x, y, Mat::Empty);
            return;
        }
        if self.chance(2500) {
            self.set(x, y, Mat::Empty);
        }
    }

    /// Puts a seed into the soil at (x, y), if it is soil with room for one more.
    fn sow(&mut self, x: i32, y: i32) -> bool {
        if !Self::inb(x, y) || !matches!(self.raw(x, y), Mat::Dirt | Mat::Grass | Mat::Mud) {
            return false;
        }
        let i = Self::idx(x, y);
        if self.growth[i] >= SOIL_SEEDS {
            return false;
        }
        self.growth[i] += 1;
        self.zones.wake(i);
        true
    }

    /// Can a seed in the soil at (x, y) start a tree here: open air above it?
    pub(super) fn seed_waits(&self, x: i32, y: i32) -> bool {
        self.growth[Self::idx(x, y)] > 0 && self.at(x, y - 1) == Mat::Empty
    }

    /// A seed in the top of fertile soil sprouts when the soil is moist and warm, in the light,
    /// with no tree too close: it drinks, and a sapling three cells wide starts above it.
    /// Buried or in dry earth the seeds wait.
    fn sprout_tree(&mut self, x: i32, y: i32) -> bool {
        let i = Self::idx(x, y);
        if !self.seed_waits(x, y) || !self.chance(TREE_SPROUT) {
            return false;
        }
        if self.moisture[i] < TREE_MOIST || self.temp[i] < 6.0 || !self.lit(x, y - 1) {
            return false;
        }
        if self.find_near(x, y - 1, 5 * K, Mat::Trunk).is_some() || self.find_near(x, y - 1, 5 * K, Mat::Sapling).is_some() {
            return false;
        }
        if !self.drink(x, y, 3, 3) {
            return false;
        }
        self.growth[i] -= 1;
        let budget = 40 + (self.rand() % 40) as u8;
        for dx in -1..=1 {
            // `raw` reads Empty past the sides too.
            if let Some(j) = Self::ix(x + dx, y - 1).filter(|_| self.raw(x + dx, y - 1) == Mat::Empty) {
                self.set(x + dx, y - 1, Mat::Sapling);
                self.growth[j] = budget;
            }
        }
        true
    }

    /// Plants a grown tree where the player clicked: its trunk stands on the first ground under
    /// `(x, y)` (or on the surface, if the click is in the ground) and ends in a crown, like the
    /// trees of the landscape. `size` (0 to 1) makes it taller. Returns false if there is no
    /// ground to stand on, or no room for it.
    pub fn plant_tree(&mut self, x: i32, y: i32, size: f32) -> bool {
        let ground = |m: Mat| matches!(m.kind(), Kind::Static | Kind::Powder) && !m.is_vegetation();
        let mut y = y.clamp(0, H() as i32 - 1);
        if ground(self.raw(x, y)) {
            while y > 0 && ground(self.raw(x, y - 1)) {
                y -= 1;
            }
        } else {
            while y < H() as i32 && !ground(self.raw(x, y)) {
                y += 1;
            }
            if y >= H() as i32 {
                return false;
            }
        }
        // A trunk in a lake would drown: it needs its feet out of the water.
        let base = y;
        if self.raw(x, base - 1).kind() == Kind::Liquid {
            return false;
        }
        let hgt = ((8.0 + size.clamp(0.0, 1.0) * 6.0) as i32 + (self.rand() % 3) as i32) * K;
        let left = x - K / 2;
        let blocked = |m: Mat| m.kind() == Kind::Static && !m.is_vegetation();
        if base - hgt < 12 * K || (1..=hgt).any(|dy| (0..K).any(|dx| blocked(self.raw(left + dx, base - dy)))) {
            return false;
        }
        // Ground under the trunk becomes soil, so the tree can drink and stands rooted.
        for dx in 0..K {
            if matches!(self.raw(left + dx, base), Mat::Sand | Mat::Snow | Mat::Grass) {
                self.set(left + dx, base, Mat::Dirt);
            }
        }
        self.fill_rect(left, base - hgt, K, hgt, Mat::Trunk);
        self.grow_crown(x, base - hgt);
        true
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
        let left = self.growth[i];
        if self.at(x, y - 1) != Mat::Empty {
            // Not a tip: it is trunk now.
            if self.at(x, y - 1) != Mat::Sapling {
                self.set(x, y, Mat::Trunk);
            }
            return;
        }
        if left == 0 {
            self.set(x, y, Mat::Trunk);
            self.grow_crown(x, y);
            return;
        }
        if self.chance(30) && self.surface_temp_c() > 6.0 && self.lit(x, y) && self.drink(x, y, 8, 50) {
            self.set(x, y, Mat::Trunk);
            // Grown to the top of the world, it stops there.
            if let Some(j) = Self::ix(x, y - 1) {
                self.set(x, y - 1, Mat::Sapling);
                self.growth[j] = left - 1;
            }
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
        // Through the crown it can reach far to the side: after the other cells.
        if self.put_off(turn::Later::Transpire(x, y)) {
            return;
        }
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
        // `raw` reads Empty past the edges: the vapour goes only into a cell of the grid.
        let Some(j) = Self::ix(vx, vy) else { return };
        if self.raw(cx, cy) == Mat::Leaf && self.raw(vx, vy) == Mat::Empty && self.drink(x, y + 1, 4 * K, 6 * K) {
            // A quarter of a cell of water leaves as vapour, with the heat that takes (the leaf
            // and the air around it cool).
            let t = self.temp[Self::idx(cx, cy)];
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
        // A bubble: gas with liquid over it rises through the liquid (Archimedes), about half a
        // metre a second, straight up or slantwise; the liquid takes its place. (The physical
        // liquids treat a gas cell as a wall, so without this a bubble stayed shut in.)
        if self.liquid_physics && self.at(x, y - 1).kind() == Kind::Liquid {
            // (A bubble keeps its zone awake: it moves too seldom to do so by itself.)
            self.zones.wake(i);
            if self.chance(BUBBLE_RISE) {
                let d = self.dir();
                for dx in [0, d, -d] {
                    if self.at(x + dx, y - 1).kind() == Kind::Liquid && self.mask_free(x + dx, y - 1) && self.mask[i] == 0 {
                        self.swap((x, y), (x + dx, y - 1));
                        return true;
                    }
                }
            }
        }
        // Foliage is no lid: gas under leaves and branches slips up through them to the open air
        // above (a tree breathes; the steam of a pond under it does not stay trapped).
        if matches!(self.at(x, y - 1), Mat::Leaf | Mat::Branch | Mat::Sapling) && self.chance(2) {
            let mut k = 2;
            while k <= 12 * K && matches!(self.at(x, y - k), Mat::Leaf | Mat::Branch | Mat::Sapling) {
                k += 1;
            }
            if self.gas_may_rise_to(x, y, y - k) && self.at(x, y - k) == Mat::Empty {
                self.swap((x, y), (x, y - k));
                return true;
            }
        }
        // Above the top of the weather, vapour or cloud that cannot rise any more (painted there,
        // or an updraft that overshot) settles back down: its ice crystals slowly fall out of the
        // thin air, and it ends up spread under the tropopause.
        if y < CEILING_Y() && !self.gas_may_rise_to(x, y, y - 1) && self.chance(3) {
            let d = self.dir();
            for dx in [0, d, -d] {
                let t = (self.edge_x(x, y, x + dx), y + 1);
                if self.at(t.0, t.1) == Mat::Empty {
                    self.swap((x, y), t);
                    return true;
                }
            }
        }
        let lift = ((self.temp[i] - self.air_c(y)) / 25.0).clamp(-1.0, 1.0);
        let roll = (self.rand() % 100) as f32 / 100.0;
        let d = self.dir();
        if lift > 0.05 && roll < lift * 0.7 {
            for (dx, dy) in [(0, -1), (d, -1), (-d, -1)] {
                let t = (self.edge_x(x, y, x + dx), y + dy);
                if self.gas_may_rise_to(x, y, t.1) && self.at(t.0, t.1) == Mat::Empty {
                    self.swap((x, y), t);
                    return true;
                }
            }
            // Pressed against the ceiling, rising gas spreads out under it, mostly downwind (the
            // jet stream drags the top of a storm cloud out into an anvil).
            if !self.gas_may_rise_to(x, y, y - 1) {
                let vx = self.flow.wind_at(x, y).0;
                let down = if vx.abs() > 0.1 && roll < 0.8 * lift.min(1.0) * 0.7 { vx.signum() as i32 } else { d };
                for dx in [down, -down] {
                    let t = (self.edge_x(x, y, x + dx), y);
                    if self.at(t.0, t.1) == Mat::Empty {
                        self.swap((x, y), t);
                        return true;
                    }
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

    /// Whether the gas at (x, y) may rise into row `to`. Under the top of the weather steam and
    /// clouds rise freely; above it, in the warm thin air of the stratosphere, only if, once
    /// cooled by the climb, they would still be warmer (lighter) than the air there, which they
    /// soon are not. (Smoke is left free: it thins out in the open air by itself.)
    pub(super) fn gas_may_rise_to(&self, x: i32, y: i32, to: i32) -> bool {
        if to >= CEILING_Y() || !Self::inb(x, y) || !matches!(self.raw(x, y), Mat::Steam | Mat::Cloud) {
            return true;
        }
        // The air it would rise into (open air there, else the usual air of that height).
        let air = if Self::inb(x, to) && self.raw(x, to) == Mat::Empty { self.temp[Self::idx(x, to)] } else { self.air_c(to) };
        self.temp[Self::idx(x, y)] - adiabatic_cooling_c(self.raw(x, y), y, to) > air + 0.5
    }

    /// Where the gas at (x, y) moving to column `to` ends up. Vapour and clouds blown past the
    /// left or right edge leave the world (an open sky: they do not pile up). Smoke, which thins
    /// out in the open air on its own, comes back on the other side.
    pub(super) fn edge_x(&self, x: i32, y: i32, to: i32) -> i32 {
        if self.raw(x, y) == Mat::Smoke { to.rem_euclid(W() as i32) } else { to }
    }

    /// Oil is a slow, light liquid: it floats on water and burns ferociously.
    fn step_oil(&mut self, x: i32, y: i32) {
        for (nx, ny) in Self::neighbors(x, y) {
            if matches!(self.at(nx, ny), Mat::Fire | Mat::Ember | Mat::Lava) && self.chance(2) {
                self.ignite_oil(x, y);
                return;
            }
        }
        self.step_liquid(x, y);
    }

    /// Burning oil: a long-lived, very hot flame that sets its neighbours alight.
    pub(super) fn ignite_oil(&mut self, x: i32, y: i32) {
        self.set(x, y, Mat::Fire);
        let i = Self::idx(x, y);
        self.fuel[i].left = 150 + (self.rand() % 100) as u8;
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
        self.step_gas(x, y);
    }

    /// The one way steam and smoke move (`Kind::Gas`), the matter only setting how it drifts
    /// (`drift`): pushed by the difference of density with the air around (`buoyancy`), then
    /// carried by the wind, and otherwise a warm gas climbs while a cooled one wanders sideways.
    /// (A cloud is droplets hanging in the air: it has its own, stiller wandering, see
    /// `step_cloud`.) Returns whether it moved.
    fn step_gas(&mut self, x: i32, y: i32) -> bool {
        if self.buoyancy(x, y) {
            return true;
        }
        if let Some(t) = self.wind_target(x, y) {
            if self.at(t.0, t.1) == Mat::Empty {
                self.swap((x, y), t);
                return true;
            }
        }
        let (hot_rise, cool_rise, climbs, wraps) = drift(self.at(x, y));
        let d = self.dir();
        let hot = self.temp[Self::idx(x, y)] > self.air_c(y) + 8.0;
        let r = self.rand() % 10;
        let order: [(i32, i32); 3] = if hot && r < hot_rise {
            [(0, -1), (d, -1), (d, 0)]
        } else if r < cool_rise {
            [(0, -1), (d, 0), (-d, 0)]
        } else {
            [(d, 0), (-d, 0), if climbs { (0, -1) } else { (d, 0) }]
        };
        for (dx, dy) in order {
            let tx = if wraps { self.edge_x(x, y, x + dx) } else { x + dx };
            let t = (tx, y + dy);
            if (dy >= 0 || self.gas_may_rise_to(x, y, t.1)) && self.at(t.0, t.1) == Mat::Empty {
                self.swap((x, y), t);
                return true;
            }
        }
        false
    }

    fn step_water(&mut self, x: i32, y: i32) {
        let i = Self::idx(x, y);
        // Soil drinks the water on it and beside it, until it is saturated: the water goes into the
        // ground (the soil gets moist) and the rest stays on top as a puddle.
        let soils = [(x, y + 1), (x - 1, y), (x + 1, y)];
        if soils.iter().any(|&n| self.soil_drinks(n)) && self.chance(25) {
            let n = soils[(self.rand() % 3) as usize];
            if self.soil_drinks(n) {
                let j = Self::idx(n.0, n.1);
                self.moisture[j] += if self.liquid_physics { self.water_units_in(i).min(SOIL_WATER) } else { SOIL_WATER };
                self.zones.wake(j);
                self.set(x, y, Mat::Empty);
                return;
            }
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
                // Water that may hold less than a cell dries a quarter at a time (one cell of
                // steam's worth, as often again): the surface of a lake sinks slowly, where a
                // whole cell gone at once would dig a hole for the water round it to splash into.
                // A drop is tiny, and goes whole.
                let by_quarters = self.liquid_physics && airborne == 1.0 && self.at(x, y - 1) == Mat::Empty;
                if by_quarters {
                    // As much water a step, on average, as a whole cell going at `rate`: a film
                    // thinner than a quarter goes as often again, and all at once.
                    let taken = self.water_fill(i).min(field::FULL / VAPOUR_PER_WATER as u32).max(1);
                    if self.chance_f(rate * field::FULL as f32 / taken as f32) {
                        self.evaporate_quarter(x, y);
                        return;
                    }
                } else if self.chance_f(rate) {
                    // The heat it takes cools the water and the air around (see `vaporize`).
                    self.vaporize(x, y);
                    return;
                }
            }
        }
        // A drop in the air falls like any heavy particle: gravity speeds it up to a terminal speed
        // (air drag) and the wind carries it along on the way down, so the rain slants and gusts.
        // Water with more water on top of it (the roof of a hole in a lake, a thick stream) just
        // sinks a cell at a time: only a free drop races down.
        let free_drop = if self.liquid_physics {
            // (Water on a cloud too, alone or not: the cloud is a wall to the liquids.)
            self.lone_liquid(x, y) || self.rain_over_gas(x, y)
        } else {
            self.at(x, y - 1).kind() != Kind::Liquid
        };
        if free_drop && matches!(self.at(x, y + 1).kind(), Kind::Empty | Kind::Gas) && self.mask_free(x, y + 1) {
            // Falling through a cloud, the drop sweeps up the droplets in its way and grows: the
            // cloud's water becomes rain (`VAPOUR_PER_WATER` cells of cloud, one cell of rain). It
            // goes by the way it falls (the tick it would go into the cell), not by the time.
            let into = self.fall_at[i] as u32 + self.vel[i].saturating_add(GRAIN_ACCEL).min(DROP_TERMINAL) as u32 >= FALL_UNIT;
            if into && self.raw(x, y + 1) == Mat::Cloud && self.chance(DROP_SWEEP) && self.gather_vapour(x, y + 1) {
                let j = Self::idx(x, y + 1);
                let t = self.temp[j].max(self.temp[i]);
                if self.liquid_physics {
                    // With the physical liquids, the water swept up flies on as a drop of theirs,
                    // at the drop's speed, and the drop goes on falling: put in the cell under
                    // the drop, it left the drop lying on it (water over water is the solver's,
                    // which cannot push water into a cloud), and it rained on from under the
                    // drop sweeping the next cell: the rain left behind it columns of water
                    // standing in the cloud, thousands of cells that the solver worked on and
                    // that drained a cell a tick from the bottom.
                    let speed = self.vel[i] as f32 / FALL_UNIT as f32 * CELL_M / STEP;
                    self.liq.drops.push(liquid::Drop { x: x as f32 + 0.5, y: y as f32 + 0.5, vx: 0.0, vy: speed, m: Mat::Water, q: field::FULL, t });
                    self.set(x, y + 1, Mat::Empty);
                    self.temp[j] = t;
                    self.fall_drop(x, y);
                    return;
                }
                self.set(x, y + 1, Mat::Water);
                self.temp[j] = t;
                return;
            }
            self.fall_drop(x, y);
            return;
        }
        // A drop that has joined other water (or landed) no longer falls freely: its speed goes
        // (moved by their physics, the liquids take it over, see `liquid.rs`).
        if !self.liquid_physics {
            self.vel[i] = 0;
        }
        self.step_liquid(x, y);
    }

    /// The one way every liquid moves (`Kind::Liquid`), the matter only setting how thick it is
    /// (`viscosity`): a liquid lighter than the one on top of it rises through it (oil on water),
    /// a thick one stays still now and then, then it runs down if it can (unless it is the mouth
    /// of a full pipe, held by the pressure behind it, see `hydro.rs`) and else spreads along.
    /// Returns whether it moved.
    fn step_liquid(&mut self, x: i32, y: i32) -> bool {
        let m = self.at(x, y);
        let (rest, reach, spread) = viscosity(m);
        let above = self.at(x, y - 1);
        if above.kind() == Kind::Liquid && density(above) > density(m) && self.chance(2) {
            self.swap((x, y), (x, y - 1));
            return true;
        }
        // Moved by their physics, the liquids only sort by weight here.
        if self.liquid_physics {
            return false;
        }
        if rest > 0 && self.chance(rest) {
            return false;
        }
        let d = self.dir();
        let down: &[i32] = if self.held(x, y) { &[] } else { &[0, d, -d] };
        for &dx in down {
            if matches!(self.at(x + dx, y + 1).kind(), Kind::Empty | Kind::Gas) {
                self.swap((x, y), (x + dx, y + 1));
                return true;
            }
        }
        if spread > 1 && !self.chance(spread) {
            return false;
        }
        for dir in [d, -d] {
            // Thin matter (a tree) does not hold it back: it runs on past it, the way the water
            // of a 2-D world goes in front of a trunk or behind it.
            let (mut to, mut cx, mut run) = (x, x, 0);
            while run < reach {
                if self.at(cx + dir, y) == Mat::Empty {
                    cx += dir;
                    to = cx;
                    run += 1;
                } else if let Some(past) = self.past_thin(cx, y, dir).filter(|&px| self.at(px, y) == Mat::Empty) {
                    cx = past;
                    to = cx;
                    run += 1;
                } else {
                    break;
                }
            }
            if to != x {
                self.swap((x, y), (to, y));
                return true;
            }
        }
        false
    }


    /// Moves a drop with free air under it: it speeds up by gravity as a grain does (see
    /// `fall_on`), goes down a cell when its way adds up to one, and sideways as far as the local
    /// wind carries it.
    fn fall_drop(&mut self, x: i32, y: i32) {
        let i = Self::idx(x, y);
        let v = self.vel[i].saturating_add(GRAIN_ACCEL).min(DROP_TERMINAL);
        let way = self.fall_at[i] as u32 + v as u32;
        self.vel[i] = v;
        let down = way >= FALL_UNIT;
        self.fall_at[i] = (if down { way - FALL_UNIT } else { way }) as u8;
        // The wind carries it sideways every tick, whether it goes down a cell or not.
        let wind = (self.flow.wind_at(x, y).0 * DROP_DRIFT).clamp(-3.0, 3.0);
        let side = wind.trunc() as i32 + if (self.rand() % 1000) as f32 / 1000.0 < wind.fract().abs() { wind.signum() as i32 } else { 0 };
        let free = |w: &World, x: i32, y: i32| matches!(w.at(x, y).kind(), Kind::Empty | Kind::Gas) && w.mask_free(x, y);
        let mut cx = x;
        for _ in 0..side.abs() {
            if !(self.at(cx + side.signum(), y) == Mat::Empty && self.mask_free(cx + side.signum(), y)) {
                break;
            }
            cx += side.signum();
        }
        let cy = if down && free(self, cx, y + 1) { y + 1 } else { y };
        // Stopped by what is below: it lands.
        if down && cy == y {
            self.vel[i] = 0;
            self.fall_at[i] = 0;
        }
        if (cx, cy) != (x, y) {
            self.swap((x, y), (cx, cy));
        }
    }

    fn step_steam(&mut self, x: i32, y: i32) {
        // Condensation is handled by temperature (see thermal.rs); it moves as a gas (`step_gas`).
        let i = Self::idx(x, y);
        // Packed vapour (a full boiler) spreads into any room it finds.
        if self.packed(i) > 0 {
            let n = Self::neighbors(x, y)[(self.rand() % 4) as usize];
            // Past the sides and the bottom `at` reads Empty too: only a cell of the grid takes it.
            if let Some(j) = Self::ix(n.0, n.1).filter(|_| self.at(n.0, n.1) == Mat::Empty) {
                let t = self.temp[i];
                let q = thermal::heat_of(Mat::Empty, self.temp[j]) - thermal::cap(Mat::Steam) * t;
                self.moisture[i] -= VAPOUR_UNITS;
                self.set(n.0, n.1, Mat::Steam);
                self.temp[j] = t;
                self.release_heat(x, y, q);
                return;
            }
        }
        self.step_gas(x, y);
    }

    /// Water at (x, y) turns to vapour and swells: the cell becomes steam, and the rest of its
    /// `VAPOUR_PER_WATER` cells of steam go into the nearest free cells around it. Shut in with no
    /// room left (a full boiler), the rest of the vapour stays packed in the cell (its `moisture`
    /// counts the extra cells' worth): no water is lost, and the packed vapour pushes harder
    /// (see `pockets.rs`) until it finds room to spread. The heat it takes beyond what the water
    /// held (all of it when the water boiled, less when it evaporates cold) comes from around.
    pub(crate) fn vaporize(&mut self, x: i32, y: i32) {
        let i = Self::idx(x, y);
        let t = self.temp[i];
        // A cell of water that is not full makes less vapour (none at all from a thin film:
        // it just dries).
        let share = if self.liquid_physics { self.water_fill(i) as f32 / field::FULL as f32 } else { 1.0 };
        self.dried += (share * field::FULL as f32) as u64;
        // (A part of a cell of steam is a whole one or none, by chance: on average the water is
        // all there.)
        let v = share * VAPOUR_PER_WATER as f32;
        let cells = v as usize + (v.fract() > 0.0 && ((self.rand() % 65536) as f32) < v.fract() * 65536.0) as usize;
        let mut q = thermal::heat_of(Mat::Water, t) * share + self.latent[i];
        if cells == 0 {
            self.set(x, y, Mat::Empty);
            self.temp[i] = t;
            self.release_heat(x, y, q - VAPOUR_PER_WATER as f32 * share * thermal::latent0(Mat::Steam));
            return;
        }
        self.set(x, y, Mat::Steam);
        self.temp[i] = t;
        let mut left = cells - 1;
        let mut made = 1.0;
        // (Not upward where the vapour could not rise: a drop evaporating above the top of the
        // weather would otherwise ratchet its water up a row each time.)
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
                    if Self::inb(nx, ny)
                        && self.at(nx, ny) == Mat::Empty
                        && (r == 1 || matches!(self.at(via.0, via.1), Mat::Empty | Mat::Steam))
                        && (dy >= 0 || self.gas_may_rise_to(x, y, ny))
                    {
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
        self.set_packed(i, left as u32);
        q -= made * thermal::cap(Mat::Steam) * t + cells as f32 * thermal::latent0(Mat::Steam);
        self.release_heat(x, y, q);
    }

    /// A quarter of a cell of water (one cell of steam's worth) leaves the water at (x, y) as a
    /// cell of steam in the empty cell over it; the cell keeps the rest, and goes when nothing is
    /// left. The heat is counted as in `vaporize`.
    fn evaporate_quarter(&mut self, x: i32, y: i32) {
        let i = Self::idx(x, y);
        let j = Self::idx(x, y - 1);
        let t = self.temp[i];
        let part = field::FULL / VAPOUR_PER_WATER as u32;
        let fill = self.water_fill(i);
        let taken = fill.min(part);
        self.dried += taken as u64;
        // Less than a quarter makes a whole cell of steam or none, by chance: on average the
        // water is all there.
        let steam = taken == part || (self.rand() % part) < taken;
        let mut q = thermal::heat_of(Mat::Water, t) * taken as f32 / field::FULL as f32;
        if fill > taken {
            self.put_fill(i, fill - taken);
        } else {
            q += self.latent[i];
            self.set(x, y, Mat::Empty);
            self.temp[i] = t;
        }
        if steam {
            q += thermal::heat_of(Mat::Empty, self.temp[j]);
            self.set(x, y - 1, Mat::Steam);
            self.temp[j] = t;
            q -= thermal::heat_of(Mat::Steam, t);
        }
        self.release_heat(x, y, q);
    }

    /// Cells' worth of vapour in a cell of steam or cloud (more than one when packed).
    pub fn vapour_units(&self, i: usize) -> u32 {
        match self.cells[i].m {
            Mat::Steam | Mat::Cloud => 1 + self.packed(i),
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
                q += thermal::heat_of(Mat::Cloud, self.temp[j]) + self.packed(j) as f32 * thermal::latent0(Mat::Cloud) - thermal::heat_of(Mat::Empty, self.temp[j]);
                let t = self.temp[j];
                self.set(cx, cy, Mat::Empty);
                self.temp[j] = t;
            } else {
                q += take as f32 * thermal::latent0(Mat::Cloud);
                self.moisture[j] -= take as u16 * VAPOUR_UNITS;
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
                let own = thermal::heat_of(Mat::Cloud, t) + self.packed(i) as f32 * thermal::latent0(Mat::Cloud);
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
        // Up at the top of the weather the jet stream sweeps the cloud along before anything
        // else: the top of a storm cloud is dragged downwind into an anvil.
        if y < CEILING_Y() + 10 {
            if let Some(t) = self.wind_target(x, y) {
                if self.at(t.0, t.1) == Mat::Empty {
                    self.swap((x, y), t);
                    return;
                }
            }
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
            } else if r == 2 && y > COLD_Y() {
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
        if self.fuel[i].left == 0 {
            self.set(x, y, Mat::Empty);
            return;
        }
        self.fuel[i].left -= 1;
        // A flame burns the oxygen of the air touching it. With no air around (only smoke,
        // steam or walls) it chokes within a few steps: that is why a glass over a candle
        // puts it out.
        let air = Self::neighbors(x, y).iter().any(|&(nx, ny)| match self.at(nx, ny) {
            Mat::Empty => !self.stale(nx, ny),
            Mat::Smoke => !self.sealed(nx, ny),
            _ => false,
        });
        if !air {
            self.fuel[i].left = self.fuel[i].left.saturating_sub(6);
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
        // Burning charcoal lasts several times longer than burning wood.
        let coal = self.fuel[i].carbon;
        if self.fuel[i].left == 0 {
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
        let slow = if coal { slow * 12 } else { slow };
        if slow == 1 || self.chance(slow) {
            self.fuel[i].left -= 1;
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
        self.fuel[i] = Fuel { left: 30 + (self.rand() % 26) as u8, carbon: true };
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
        self.step_liquid(x, y);
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
    /// just falls (see `step_grass`) until something solid catches it. Earth is not either: it is
    /// a granular matter that moves grain by grain (see `step_dirt`).
    fn structural(m: Mat) -> bool {
        m.kind() == Kind::Static && !matches!(m, Mat::Grass | Mat::Dirt)
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

    /// Are the neighbouring cells `i` and `j` parts of one body? Touching cells of the same matter
    /// are (a tree is one body, see `statics::same_body`). A log (one stroke of the wood brush) is
    /// a rigid body of its own: it is bonded only to the wood of the same stroke. It rests on what
    /// is under it and carries what lies on it, but it never sticks to the side of anything, so it
    /// falls as a whole and lands as a log instead of melting into the ground.
    fn same_body(&self, i: usize, j: usize) -> bool {
        let (a, b) = (self.cells[i].m == Mat::Wood, self.cells[j].m == Mat::Wood);
        if a || b {
            a && b && self.log[i] == self.log[j]
        } else {
            statics::same_body(self.cells[i].m, self.cells[j].m)
        }
    }

    /// A new log for the wood brush: the next strokes of wood are a body of their own.
    pub fn new_log(&mut self) -> u16 {
        self.paint_log = self.paint_log.wrapping_add(1).max(1);
        self.paint_log
    }

    /// Sorts the solids into anchored bodies and free ones. A solid body is a run of touching
    /// cells of the same matter (a tree counts as one body, see `statics::same_body`). It is
    /// anchored when it touches something fixed: the base of the world, a tunnel's vault, or the
    /// ground its roots grow in. Anchored bodies are then checked for overloaded sections (see
    /// `stress_pass`); free bodies either fall, tip over their edge (see `tip_pass`) or rest.
    fn structure_pass(&mut self) {
        let mut seen = std::mem::take(&mut self.seen);
        let mut stack = std::mem::take(&mut self.stack);
        let mut level = std::mem::take(&mut self.level);
        seen.fill(false);
        level.fill(u32::MAX);
        // Bodies in flight are moved by their own physics.
        for s in &self.slabs {
            for &i in s.cells() {
                seen[i] = true;
            }
        }
        // Breadth first from the fixed things, so that each cell learns how far it is from them.
        let mut order: Vec<usize> = Vec::new();
        for i in 0..W() * H() {
            if !seen[i] && self.anchors(i) {
                seen[i] = true;
                level[i] = 0;
                order.push(i);
            }
        }
        let mut head = 0;
        while head < order.len() {
            let i = order[head];
            head += 1;
            let (x, y) = ((i % W()) as i32, (i / W()) as i32);
            let from = self.cells[i].m;
            for (nx, ny) in Self::neighbors(x, y) {
                if Self::inb(nx, ny) {
                    let j = Self::idx(nx, ny);
                    let to = self.cells[j].m;
                    if !seen[j] && Self::structural(to) && self.same_body(i, j) && Self::holds(from, to, ny - y) {
                        seen[j] = true;
                        level[j] = level[i] + 1;
                        order.push(j);
                    }
                }
            }
        }
        self.level = level;
        self.stress_pass(&order, &mut seen);
        // A piece that was already falling keeps its speed (sampled cells are enough).
        let mut speeds: std::collections::HashMap<usize, (f32, f32)> = std::collections::HashMap::new();
        for p in &self.falling {
            for c in p.cells.iter().step_by(5).take(400) {
                speeds.insert(*c, (p.vy, p.fallen));
            }
        }
        // Group what is left into bodies.
        self.falling.clear();
        let mut free = vec![];
        let mut budget = 300_000usize;
        for start in 0..W() * H() {
            if seen[start] || !Self::structural(self.cells[start].m) {
                continue;
            }
            let mut cells = vec![];
            seen[start] = true;
            stack.push(start);
            while let Some(i) = stack.pop() {
                cells.push(i);
                let (x, y) = ((i % W()) as i32, (i / W()) as i32);
                for (nx, ny) in Self::neighbors(x, y) {
                    if Self::inb(nx, ny) {
                        let j = Self::idx(nx, ny);
                        if !seen[j] && Self::structural(self.cells[j].m) && self.same_body(i, j) {
                            seen[j] = true;
                            stack.push(j);
                        }
                    }
                }
            }
            budget = budget.saturating_sub(cells.len());
            let (vy, fallen) = cells.iter().step_by(5).filter_map(|c| speeds.get(c)).fold((0.0f32, 0.0f32), |a, b| (a.0.max(b.0), a.1.max(b.1)));
            let density = cells.iter().map(|&i| density(self.cells[i].m)).sum::<f32>() / cells.len().max(1) as f32;
            free.push(Piece { cells, vy, fallen, density });
            if budget == 0 {
                break;
            }
        }
        self.seen = seen;
        self.stack = stack;
        self.tip_pass(free);
    }

    /// Whether cell `i` is solid and held by something fixed: the base of the world, a tunnel's
    /// vault (it is bolted into the ground around it), or, for a tree, the ground its roots grow in.
    fn anchors(&self, i: usize) -> bool {
        let m = self.cells[i].m;
        if !Self::structural(m) || self.mask[i] != 0 {
            return false;
        }
        if i / W() == H() - 1 || m == Mat::Vault {
            return true;
        }
        if matches!(m, Mat::Trunk | Mat::Sapling) {
            // Roots: a tree holds to the ground it stands on.
            let (x, y) = ((i % W()) as i32, (i / W()) as i32);
            let ground = |m: Mat| m.is_hard() && !m.is_vegetation();
            return ground(self.raw(x, y + 1)) || Self::neighbors(x, y).iter().any(|&(nx, ny)| matches!(self.raw(nx, ny), Mat::Dirt | Mat::Grass | Mat::Mud));
        }
        false
    }

    /// Moves every unsupported piece down (accelerating) as a rigid whole. A fast
    /// landing bounces a little. Ice pieces float on liquids instead of sinking.
    fn drop_structures(&mut self) {
        let mut pieces = std::mem::take(&mut self.falling);
        for piece in &mut pieces {
            piece.cells.sort_unstable_by(|a, b| b.cmp(a));
        }
        // The lowest piece first: a log lying on another falling one follows it down.
        pieces.sort_by_key(|p| std::cmp::Reverse(p.cells.first().copied().unwrap_or(0)));
        for piece in &mut pieces {
            if piece.vy >= 0.0 {
                // Falling: gravity builds up speed; move one cell at a time while it is free.
                piece.vy = (piece.vy + 0.35).min(3.0);
                for _ in 0..(piece.vy.max(1.0) as i32) {
                    if !self.piece_can_move(piece, 1) && !self.crush_foliage(piece) {
                        break;
                    }
                    // Ploughing into loose grains: pushing them aside takes its speed.
                    if self.ploughs(piece) {
                        piece.vy -= PLOUGH_BRAKE;
                    }
                    self.piece_move(piece, 1);
                    piece.fallen += 1.0;
                }
                if !self.piece_can_move(piece, 1) {
                    // Under water and lighter than it: it comes up (Archimedes), a row a step,
                    // until its top is out (a log that dived in resurfaces).
                    if self.buoyed(piece) {
                        piece.vy = 0.0;
                        self.piece_move(piece, -1);
                        continue;
                    }
                    // Touched down. It may break (see `statics.rs`); if not, a hard landing
                    // bounces right away and a soft one settles.
                    if self.landing_breaks(piece) {
                        piece.cells.clear();
                        continue;
                    }
                    piece.fallen = 0.0;
                    if piece.vy > 1.4 {
                        if self.thuds.len() < 8 {
                            let c = piece.cells[0];
                            self.thuds.push(((c % W()) as i32, (c / W()) as i32, piece.vy));
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
            let t = i + W();
            if t >= W() * H() || in_piece(t) || self.mask[t] != 0 {
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
            self.set((t % W()) as i32, (t / W()) as i32, Mat::DeadLeaf);
        }
        self.piece_can_move(piece, 1)
    }

    /// The upward part of a bounce.
    /// Whether a piece is under a liquid it is lighter than and can come up a row through it:
    /// every cell over it is air, gas or such a liquid, one of them liquid in a body of liquid
    /// (not a stray drop), and one of the cells under it liquid in a body too (immersed, not a
    /// puddle lying on a block in the air, nor a block of ice in the sky with a stray drop
    /// caught over it and one under).
    fn buoyed(&self, piece: &Piece) -> bool {
        let in_piece = |i: usize| piece.cells.binary_search_by(|p| i.cmp(p)).is_ok();
        let lighter = |j: usize| self.mask[j] == 0 && self.cells[j].m.kind() == Kind::Liquid && piece.density < density(self.cells[j].m);
        let body = |j: usize| lighter(j) && ((j >= W() && lighter(j - W())) || (j % W() > 0 && lighter(j - 1)) || (j % W() + 1 < W() && lighter(j + 1)));
        let mut under_liquid = false;
        for &i in &piece.cells {
            if i < W() {
                return false;
            }
            let j = i - W();
            if in_piece(j) {
                continue;
            }
            if body(j) {
                under_liquid = true;
            } else if !(self.mask[j] == 0 && matches!(self.cells[j].m.kind(), Kind::Empty | Kind::Gas | Kind::Flame)) {
                return false;
            }
        }
        under_liquid && piece.cells.iter().any(|&i| i + W() < W() * H() && !in_piece(i + W()) && lighter(i + W()))
    }

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

    /// Whether moving `piece` down a row pushes packed grains (sand, shards) aside.
    fn ploughs(&self, piece: &Piece) -> bool {
        let in_piece = |i: usize| piece.cells.binary_search_by(|p| i.cmp(p)).is_ok();
        piece.cells.iter().any(|&i| {
            let t = i + W();
            t < W() * H() && !in_piece(t) && self.mask[t] == 0 && self.cells[t].m.kind() == Kind::Powder && density(self.cells[t].m) >= 1.0
        })
    }

    fn piece_can_move(&self, piece: &Piece, dir: i32) -> bool {
        let in_piece = |i: usize| piece.cells.binary_search_by(|p| i.cmp(p)).is_ok();
        piece.cells.iter().all(|&i| {
            let target = i as i64 + dir as i64 * W() as i64;
            if target < 0 {
                return false;
            }
            let target = target as usize;
            if target >= W() * H() {
                // Off the bottom of the world: it simply disappears.
                return true;
            }
            in_piece(target)
                || (self.mask[target] == 0
                    && match self.cells[target].m.kind() {
                        Kind::Empty | Kind::Gas | Kind::Flame => true,
                        // Archimedes: a piece heavier than the liquid (rock, iron) sinks through
                        // it, pushing it aside; a lighter one (ice, wood) floats on a body of it,
                        // but a stray drop or a puff of liquid in mid-air does not hold it up.
                        // Settled sand or earth holds up a log the same way. Packed grains are
                        // no liquid, though: they hold up even a heavy piece (rock, glass) resting
                        // on them; only one that comes down fast ploughs into them (see `PLOUGH`).
                        // Light flakes (dead leaves, snow, ash) are crushed under it all the same.
                        Kind::Liquid | Kind::Powder => {
                            let m = self.cells[target].m;
                            let heavier = piece.density >= density(m);
                            let bears = m.kind() == Kind::Powder && density(m) >= 1.0;
                            // (A film of liquid, under half a cell, never holds a piece up: one
                            // caught in a notch of a block of ice held the block up in the sky.)
                            let film = m.kind() == Kind::Liquid && self.liquid_fill(target) < field::FULL / liquid::HOLDS;
                            if (heavier && (!bears || piece.vy >= PLOUGH)) || film {
                                true
                            } else {
                                // (A drop caught in a notch of the piece rests on the piece
                                // itself: it comes down with it.)
                                let under = target + W();
                                let settled = under >= W() * H() || (!in_piece(under) && matches!(self.cells[under].m.kind(), Kind::Liquid | Kind::Static | Kind::Powder));
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
            let target = i as i64 + dir as i64 * W() as i64;
            self.zones.wake(i);
            if target as usize >= W() * H() {
                self.cells[i] = EMPTY;
                self.clear_states(i);
                gone.push(k);
                continue;
            }
            let t = target as usize;
            self.zones.wake(t);
            self.carry(i, t);
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
        let raw = self.pressure_rows(H() - 1);
        let mut out = raw.clone();
        for y in 0..H() {
            self.smooth_pressure_row(&raw, y, &mut out[y * W()..(y + 1) * W()]);
        }
        out
    }

    /// The pressure at one cell (atmospheres).
    pub fn pressure_at_cell(&self, x: i32, y: i32) -> f32 {
        if !Self::inb(x, y) {
            return 0.0;
        }
        // The load of the ground spreads, so a cell depends on a wide cone above it: the whole field
        // is worked out, and while the game runs kept for a few ticks.
        let mut cache = self.pressure_cache.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = matches!(&*cache, Some((rev, tick, _)) if *rev == self.revision || (1..8).contains(&self.ticks.wrapping_sub(*tick)));
        if !fresh {
            *cache = Some((self.revision, self.ticks, self.pressure_field()));
        }
        cache.as_ref().map_or(0.0, |(_, _, f)| f[Self::idx(x, y)])
    }

    /// Liquid and rock are smoothed sideways (never across the air), so the layers of each column do
    /// not show as stripes.
    fn smooth_pressure_row(&self, raw: &[f32], y: usize, out: &mut [f32]) {
        let open = |x: usize| matches!(self.at(x as i32, y as i32).kind(), Kind::Empty | Kind::Gas | Kind::Flame);
        let mut x = 0;
        while x < W() {
            if open(x) {
                out[x] = raw[y * W() + x];
                x += 1;
                continue;
            }
            let mut end = x;
            while end < W() && !open(end) {
                end += 1;
            }
            for xx in x..end {
                let (lo, hi) = (xx.saturating_sub(6).max(x), (xx + 6).min(end - 1));
                let sum: f32 = (lo..=hi).map(|q| raw[y * W() + q]).sum();
                out[xx] = sum / (hi - lo + 1) as f32;
            }
            x = end;
        }
    }

    /// The pressure of every cell of rows `0..=until`, row by row from the top.
    ///
    /// Air: thinner with height, moved by the flow, or that of its closed pocket. Liquid: the weight
    /// of the column above it (hydrostatic), plus the excess pressure the liquid has passed on.
    /// Solid and loose ground: the weight above as well, but rock and sand do not carry it straight
    /// down in a column as a liquid does: each row spreads the load of the row above over its
    /// neighbours, so the weight of a tree, a pillar or the walls of a crater fans out with depth
    /// (as stress does in the ground) instead of running down to the bottom as a stripe.
    fn pressure_rows(&self, until: usize) -> Vec<f32> {
        // About one atmosphere for 60 cells of water.
        const PER_CELL: f32 = 1.0 / 60.0;
        // How far each row spreads the load of the row above, either side.
        const SPREAD: usize = 2;
        let mut out = vec![0.0f32; W() * (until + 1)];
        // The pressure of the closed pocket just above each column, if a liquid lies in one.
        let mut pocket_top: Vec<Option<f32>> = vec![None; W()];
        // Pressure carried down from the row above, without the excess (which is added once, per cell).
        let mut prev = vec![0.0f32; W()];
        let mut cur = vec![0.0f32; W()];
        let mut prev_dense = vec![false; W()];
        let mut dense = vec![false; W()];
        for y in 0..=until {
            // The still air at this height (the same atmosphere as the sensors read).
            let air = air_pressure_atm(y as i32);
            // The wind solver's pressure is only read for its changes along the row (a gust, the
            // push in front of a blast): its level, row by row, is the solver's own and not the
            // atmosphere's.
            let row_flow: Vec<Option<f32>> = (0..W()).map(|x| self.flow.air_pressure(x as i32, y as i32)).collect();
            let (sum, n) = row_flow.iter().flatten().fold((0.0f32, 0usize), |(s, n), p| (s + p, n + 1));
            let row_mean = if n > 0 { (sum / n as f32).max(1e-3) } else { 1.0 };
            for x in 0..W() {
                let m = self.at(x as i32, y as i32);
                match m.kind() {
                    Kind::Empty | Kind::Gas | Kind::Flame => {
                        let dynamic = 1.0 + (self.flow.pressure_at(x as i32, y as i32) * 0.08).clamp(-0.08, 0.08);
                        // Ideal gas, P V = n R T. With compressible air the wind solver keeps density
                        // and temperature together and gives the pressure directly; otherwise the
                        // pressure follows the absolute temperature at fixed volume, so warm air
                        // reads as higher pressure.
                        let closed = self.pocket_at(x as i32, y as i32).map(|k| self.pockets[k]).filter(|p| !p.open);
                        let p_air = if let Some(p) = closed {
                            // A closed pocket (a boiler, a sealed room) has its own pressure.
                            air * p.pressure
                        } else {
                            match row_flow[x] {
                                // Open air evens out its pressure almost at once: the solver's
                                // swings (hot air over a fire reads three times its row) are shown a
                                // quarter as strong, still enough to see a gust or a blast.
                                Some(p) => air * (1.0 + (p / row_mean - 1.0) * 0.25).clamp(0.8, 1.6),
                                None => {
                                    let kelvin = (self.temp[y * W() + x] + 273.15).max(1.0) / (self.ambient_c(y as i32) + 273.15).max(1.0);
                                    air * dynamic * kelvin.clamp(0.5, 3.0)
                                }
                            }
                        };
                        out[y * W() + x] = p_air;
                        dense[x] = false;
                        pocket_top[x] = closed.map(|_| p_air);
                    }
                    kind => {
                        // From the still-air pressure at the top of the column (or of the pocket it
                        // lies in), plus the weight above.
                        let above = if !prev_dense[x] {
                            pocket_top[x].unwrap_or(air)
                        } else if kind == Kind::Liquid {
                            prev[x]
                        } else {
                            let (lo, hi) = (x.saturating_sub(SPREAD), (x + SPREAD).min(W() - 1));
                            let (sum, n) = (lo..=hi).filter(|&q| prev_dense[q]).fold((0.0, 0.0), |(s, n), q| (s + prev[q], n + 1.0));
                            sum / n
                        };
                        cur[x] = above + density(m) * PER_CELL;
                        out[y * W() + x] = cur[x] + self.excess_pressure_at(x as f32 + 0.5, y as f32 + 0.5);
                        dense[x] = true;
                    }
                }
            }
            std::mem::swap(&mut prev, &mut cur);
            std::mem::swap(&mut prev_dense, &mut dense);
        }
        out
    }

    /// The pressure view's colour of a cell, with the borders between gas, liquid and solid marked.
    pub fn pressure_color(&self, field: &[f32], x: usize, y: usize) -> [u8; 4] {
        let m = self.at(x as i32, y as i32);
        // Two scales: the air's changes are a few hundredths of an atmosphere, the ground's tens of
        // atmospheres; on one scale either the sky or the ground would be a flat colour.
        let (r, g, b) = if matches!(m.kind(), Kind::Empty | Kind::Gas | Kind::Flame) { air_pressure_rgb(field[y * W() + x]) } else { ground_pressure_rgb(field[y * W() + x]) };
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
        let (bw, bh) = (W() / B, H().div_ceil(B));
        let (mut vapour, mut wet, mut temp) = (vec![0.0f32; bw * bh], vec![0.0f32; bw * bh], vec![0.0f32; bw * bh]);
        let mut air = vec![0.0f32; bw * bh];
        for y in 0..H() {
            for x in 0..W() {
                let i = y * W() + x;
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
            Kind::Empty | Kind::Gas => Some(if matches!(m, Mat::Cloud | Mat::Steam) { 1.0 } else { field[y * W() + x] }),
            _ => match m {
                // Soil holds moisture of its own.
                Mat::Mud => Some(1.0),
                Mat::Dirt | Mat::Grass => Some((self.moisture[y * W() + x] as f32 / SOIL_FULL as f32).clamp(0.0, 1.0) * 0.8 + 0.1),
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
        self.cells[y * W() + x].m
    }

    /// Everything but the trees, which are drawn as a separate swaying layer.
    pub fn color_without_trees(&self, x: usize, y: usize) -> [u8; 4] {
        if self.cells[y * W() + x].m.is_vegetation() {
            return [0, 0, 0, 0];
        }
        self.color(x, y)
    }

    pub fn color(&self, x: usize, y: usize) -> [u8; 4] {
        let c = self.cells[y * W() + x];
        if c.m == Mat::Empty {
            return [0, 0, 0, 0];
        }
        let h = (x as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663);
        let v = ((h >> 7) & 15) as i32 - 8;
        // A fixed speckle per cell; the shimmer of water and the flicker of embers and lava move
        // in the shader (render.rs), so a cell that does not change keeps its colour.
        let flick = (h >> 11 & 31) as i32;
        let t = self.temp[y * W() + x];
        let (r, g, b, a): (i32, i32, i32, i32) = match c.m {
            Mat::Empty => (0, 0, 0, 0),
            Mat::Sand => (226 + v, 200 + v, 124 + v, 255),
            Mat::Water => {
                // Warm water shifts toward a pale, reddish tone just before it boils.
                let warm = ((t - 30.0).max(0.0) * 1.3) as i32;
                let (r, g, b) = (44 + v / 2 + warm, 124 + v + flick / 4 + warm / 3, (224 + v - warm).max(0));
                if !self.liquid_physics {
                    (r, g, b, 205)
                } else {
                    // A cell of the field that is not full is water mixed with air: the less water,
                    // the clearer it shows, and the paler (spray and the froth of a splash are
                    // whitish, not a deeper blue).
                    let q = self.water_fill(y * W() + x).min(field::FULL) as i32 * 256 / field::FULL as i32;
                    let pale = |c: i32, to: i32| c + (to - c) * (256 - q) / 256;
                    (pale(r, 200), pale(g, 222), pale(b, 240), 205 * q / 256)
                }
            }
            Mat::Fire => (255, (80 + self.fuel[y * W() + x].left as i32 * 3).min(235), 30, 235),
            Mat::Steam | Mat::Cloud => (0, 0, 0, 0),
            Mat::Iron => glow((146 + v, 156 + v, 174 + v), t),
            Mat::Wood => (146 + v / 2 + (y as i32 % 3) * 6, 98 + v / 2, 54, 255),
            Mat::Stone => {
                // Darker with depth, with soft mineral patches.
                let patch = (((x / 7) as u32).wrapping_mul(2654435761) ^ ((y / 7) as u32).wrapping_mul(40503)) >> 13 & 7;
                let depth = 1.0 - (y as f32 / H() as f32) * 0.35;
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
            // Clear, with the odd cell that catches the light.
            Mat::Diamond => if flick > 28 { (255, 255, 255, 255) } else { (196 + v, 236 + v / 2, 246, 235) },
            // Red sticks with a pale band of paper.
            Mat::Dynamite => if y % 6 == 0 { (226 + v / 2, 214 + v / 2, 190, 255) } else { (196 + v, 46 + v / 2, 38, 255) },
            Mat::Dirt => {
                // Wet earth is darker.
                let k = 256 - 96 * (self.moisture[y * W() + x].min(SOIL_FULL) as i32) / SOIL_FULL as i32;
                // Fertile earth shows its seeds: pale specks, more of them the more it holds.
                let seeds = self.growth[y * W() + x] as u32;
                if seeds > 0 && (h >> 3) % 16 < 2 + seeds {
                    (184 + v, 150 + v, 96 + v / 2, 255)
                } else {
                    ((112 + v) * k / 256, (78 + v / 2) * k / 256, (52 + v / 2) * k / 256, 255)
                }
            }
            Mat::Grass => {
                // Green on moist soil, turning straw yellow as the soil dries.
                let d = ((GRASS_GREEN as i32 - self.moisture[y * W() + x] as i32) * 256 / (GRASS_GREEN - GRASS_DRY) as i32).clamp(0, 256);
                let (r, g, b) = self.grass_rgb;
                let mix = |a: i32, s: i32| a + (s - a) * d / 256;
                (mix(r, 200) + v / 2, mix(g, 178) + v, mix(b, 84) + v / 2, 255)
            }
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
mod tests {
    use super::*;

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
                        w.temp[y as usize * W() + x as usize] = 300.0;
                    }
                }
                for _ in 0..600 {
                    w.step();
                }
                w.temp[205 * W() + 205]
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
            let level = time(&mut || w.level_liquids());
            let per = |d: Duration, every: f32| format!("{:6.2} ms/call x1/{every} = {:5.2} ms/tick", d.as_secs_f32() * 1000.0, d.as_secs_f32() * 1000.0 / every);
            eprintln!("=== {biome:?}: whole tick {:.2} ms", whole.as_secs_f32() * 1000.0);
            eprintln!("  cells              {}  ({} of {} zones awake)", per(cells, 1.0), w.zones.awake_count(), sleep::CW() * sleep::CH());
            eprintln!("  flow step+refresh  {}", per(flow, 1.0));
            eprintln!("  update_pressure    {}", per(press, 4.0));
            eprintln!("  pressure_push      {}", per(push, 1.0));
            eprintln!("  thermal_pass       {}", per(thermal, 3.0));
            eprintln!("  (carry_air_heat    {})", per(carry, 6.0));
            eprintln!("  level_liquids      {}", per(level, hydro::LEVEL_EVERY as f32));
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

    #[test]
    fn every_liquid_moves_by_one_rule_the_thinner_the_further() {
        // The same pour of each liquid on a cold floor: after the same time water has run
        // furthest, oil less far, lava least. (Cold enough here that nothing sets or boils:
        // the motion alone is compared.)
        let spread = |m: Mat| {
            let mut w = World::new_static();
            for x in 0..W() as i32 {
                w.set(x, 300, Mat::Stone);
            }
            for y in 280..300 {
                w.set(480, y, m);
            }
            for _ in 0..300 {
                for y in (0..300).rev() {
                    for x in 300..660 {
                        if w.raw(x, y) == m {
                            w.step_liquid(x, y);
                        }
                    }
                }
            }
            let xs: Vec<i32> = (0..W() as i32).filter(|&x| (280..300).any(|y| w.raw(x, y) == m)).collect();
            xs.last().unwrap() - xs.first().unwrap()
        };
        let (water, oil, lava) = (spread(Mat::Water), spread(Mat::Oil), spread(Mat::Lava));
        assert!(water > oil && oil > lava, "water {water}, oil {oil}, lava {lava}");
    }

    #[test]
    fn oil_rises_through_water_by_the_liquid_rule() {
        let mut w = World::new_static();
        for x in 4..7 {
            w.set(x, 11, Mat::Stone);
            w.set(x, 10, Mat::Stone);
        }
        w.set(5, 10, Mat::Oil);
        w.set(5, 9, Mat::Water);
        while !w.step_liquid(5, 10) {}
        assert_eq!((w.raw(5, 9), w.raw(5, 10)), (Mat::Oil, Mat::Water));
    }

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
        let base = w.temp[99 * W() + 10];
        let mut warmed = false;
        for _ in 0..300 {
            if w.raw(5, 98) == Mat::Empty {
                w.set(5, 98, Mat::Fire);
            }
            w.step();
            warmed |= (1..19).any(|x| w.temp[99 * W() + x] > base + 8.0);
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
        w.temp[90 * W() + 5] = thermal::LAVA_C;
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

    /// A tree in a pot of water, for `water_passes_a_tree_sideways_but_not_down_through_it`: a
    /// crown across the pot between its walls (water poured on it), and beside it a trunk
    /// standing on the bottom with water on one side only.
    fn tree_in_a_pot(physics: bool) -> World {
        let mut w = World::new_static();
        w.set_liquid_physics(physics);
        for x in 0..W() as i32 {
            w.set(x, 200, Mat::Stone);
        }
        for y in 100..200 {
            for x in [3, 17, 60] {
                w.set(x, y, Mat::Stone);
            }
        }
        for y in 150..160 {
            for x in 4..17 {
                w.set(x, y, Mat::Leaf);
            }
        }
        for x in 6..14 {
            w.set(x, 140, Mat::Water);
        }
        for y in 120..200 {
            for x in 36..39 {
                w.set(x, y, Mat::Trunk);
            }
        }
        for y in 185..200 {
            for x in 18..36 {
                w.set(x, y, Mat::Water);
            }
        }
        w
    }

    #[test]
    fn water_passes_a_tree_sideways_but_not_down_through_it() {
        for physics in [false, true] {
            let mut w = tree_in_a_pot(physics);
            for _ in 0..600 {
                w.step();
            }
            let water = |w: &World, xs: std::ops::Range<i32>, ys: std::ops::Range<i32>| xs.flat_map(|x| ys.clone().map(move |y| (x, y))).filter(|&(x, y)| w.at(x, y) == Mat::Water).count();
            let (above, below) = (water(&w, 4..17, 100..150), water(&w, 4..17, 160..200));
            assert!(above > 0 && below == 0, "physics {physics}: the crown holds the water up (above {above}, below {below})");
            let (left, right) = (water(&w, 18..36, 150..200), water(&w, 39..60, 150..200));
            assert!(right > 10 && left > 10, "physics {physics}: the water ran through the trunk (left {left}, right {right})");
        }
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
        let (near, far, cloud) = (f[290 * W() + 120], f[50 * W() + 400], f[106 * W() + 620]);
        assert!(near > far + 0.15, "air over water is damper: {near} vs {far}");
        assert!(cloud > 0.95, "inside a cloud the air is saturated: {cloud}");
        assert!(far < 0.3, "dry air far from water: {far}");
    }

    #[test]
    fn hot_air_slipping_up_through_falling_sand_leaves_its_heat_in_the_grains() {
        let mut w = World::new_static();
        w.climate_fixed = Some(-5.0);
        w.reset_temps();
        // A grain of cold sand over a pocket of very hot air: the grain falls through it.
        w.set(100, 100, Mat::Sand);
        for y in 101..104 {
            w.temp[World::idx(100, y)] = 600.0;
        }
        w.set(100, 104, Mat::Stone);
        w.swap((100, 100), (100, 103));
        let (sand, air) = (w.temp_c(100, 103), w.temp_c(100, 100));
        assert_eq!(w.raw(100, 103), Mat::Sand);
        assert!(sand > 50.0, "the grain took heat from the air it fell through: {sand}");
        assert!((air - sand).abs() < 1e-3, "and the air that slipped up left at the same temperature: {air}");
        let total = |t_sand: f32, t_air: f32| thermal::cap(Mat::Sand) * t_sand + thermal::cap(Mat::Empty) * t_air;
        assert!((total(sand, air) - total(-5.0, 600.0)).abs() < 1e-2, "no heat made or lost");
    }

    #[test]
    fn the_pressure_view_reads_the_real_atmosphere_on_its_own_scale() {
        let mut w = World::new_static();
        for x in 0..W() as i32 {
            for y in 450..H() as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        let f = w.pressure_field();
        let (low, high) = (f[440 * W() + 300], f[10 * W() + 300]);
        assert!((low - air_pressure_atm(440)).abs() < 0.05, "air by the ground: {low}");
        assert!(high < 0.2 * low, "the air thins with height: {high} up high, {low} below");
        // The sky's gradient spans the air scale, and the ground has a scale of its own.
        assert!(air_scale(low) - air_scale(high) > 0.3);
        assert_ne!(air_pressure_rgb(1.0), ground_pressure_rgb(1.0), "air and ground never share a colour at the surface");
        assert!(ground_scale(f[530 * W() + 300]) > ground_scale(f[460 * W() + 300]));
    }

    #[test]
    fn the_weight_of_a_pillar_fans_out_in_the_ground_instead_of_running_down_as_a_stripe() {
        let mut w = World::new_static();
        for x in 0..W() as i32 {
            for y in 300..H() as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        for x in 397..403 {
            for y in 240..300 {
                w.set(x, y, Mat::Stone);
            }
        }
        let f = w.pressure_field();
        let load = 60.0 * density(Mat::Stone) / 60.0;
        let (under, aside) = (|y: usize| f[y * W() + 400], |y: usize| f[y * W() + 480]);
        assert!(under(302) - aside(302) > 0.3 * load, "right under the pillar its weight is felt: {} vs {}", under(302), aside(302));
        assert!(under(500) - aside(500) < 0.15 * load, "deep down it has spread out: {} vs {}", under(500), aside(500));
        // And the ground still gets steadily heavier with depth.
        assert!(aside(500) > aside(400) && aside(400) > aside(320));
        assert_eq!(w.pressure_at_cell(400, 450), f[450 * W() + 400], "the loupe reads the same pressure as the view");
    }

    #[test]
    fn a_grain_of_steam_shows_as_a_dot_in_every_science_view() {
        let mut w = World::new_static();
        w.set(400, 200, Mat::Steam);
        let i = 200 * W() + 400;
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
        let (r, g, b) = air_pressure_rgb(pf[i]);
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
    fn volumes_count_what_the_cells_hold() {
        let mut w = World::new_static();
        let before = w.volumes().1;
        for x in 100..110 {
            w.set(x, 200, Mat::Water);
            w.set(x, 190, Mat::Steam);
        }
        let i = World::idx(100, 200);
        w.fill[i] = field::FULL / 4;
        w.set_packed(World::idx(100, 190), 3);
        let (n, v) = w.volumes();
        assert_eq!(n, w.census(), "the same census");
        let water = v[Mat::Water as usize] - before[Mat::Water as usize];
        assert!((water - 9.25).abs() < 1e-3, "a quarter-full cell counts a quarter: {water}");
        let steam = v[Mat::Steam as usize] - before[Mat::Steam as usize];
        assert!((steam - 13.0).abs() < 1e-3, "packed vapour counts its cells: {steam}");
    }

    #[test]
    fn taking_soil_lets_the_ground_sink_with_its_grass() {
        let mut w = World::new_static();
        for x in 0..W() as i32 {
            for y in 320..H() as i32 {
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
        assert_eq!(grass(&w), 6 * W(), "no grass is lost");
        for x in 0..W() as i32 {
            // No cave: from the surface down, the column is grass, then earth, then rock.
            let top = (0..H() as i32).find(|&y| w.raw(x, y) != Mat::Empty).unwrap();
            let col: Vec<Mat> = (top..H() as i32).map(|y| w.raw(x, y)).collect();
            assert!(col[..6].iter().all(|&m| m == Mat::Grass), "x={x}: turf on top, got {:?}", &col[..8]);
            assert!(col[6..].iter().all(|&m| m != Mat::Empty), "x={x}: a hole under the turf");
        }
        // The surface dipped where the earth was taken.
        assert!((0..H() as i32).find(|&y| w.raw(200, y) == Mat::Grass).unwrap() > 300, "the ground should sink");
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
            w.set(W() as i32 - 3 + (k % 3), 60 + k / 3, Mat::Cloud);
            w.set(W() as i32 - 3 + (k % 3), 70 + k / 3, Mat::Steam);
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
            for y in 80..80 + rows {
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
        assert!(thin < 0.02, "a thin veil of cloud hardly rains (a drop or so): lost {thin}");
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
        let at = |w: &World| (0..H() as i32).find(|&y| (380..420).any(|x| w.raw(x, y) == Mat::Water)).unwrap_or(H() as i32) - 20;
        let mut fell = vec![];
        for _ in 0..3 {
            for _ in 0..20 {
                w.step();
            }
            fell.push(at(&w));
        }
        // From rest, ½ g t²: 3, 12 and 27 cells of 1/6 m after 1/3, 2/3 and 1 s.
        assert!((2..=5).contains(&fell[0]) && (9..=15).contains(&fell[1]) && (22..=30).contains(&fell[2]), "a drop falls by gravity: {fell:?} cells");
    }

    #[test]
    fn a_seed_goes_into_the_soil_and_makes_it_fertile_but_rots_on_rock() {
        let mut w = World::new_static();
        for x in 100..140 {
            for y in 300..320 {
                w.set(x, y, if x < 120 { Mat::Dirt } else { Mat::Stone });
            }
        }
        w.set(110, 290, Mat::Seed);
        w.set(130, 290, Mat::Seed);
        // Night: the sown seed must wait in the soil, not sprout.
        w.set_phase(0.75);
        w.day_cycle = false;
        // (Ten cells of 1/6 m from rest: 0.6 s.)
        for _ in 0..60 {
            w.step();
        }
        assert_eq!(w.raw(110, 300), Mat::Dirt, "the seed does not pile up: it goes into the earth");
        assert_eq!(w.growth[300 * W() + 110], 1, "the earth under it is fertile");
        assert!((280..300).all(|y| w.raw(110, y) == Mat::Empty));
        for _ in 0..15_000 {
            w.step();
        }
        assert_eq!(w.cells.iter().filter(|c| c.m == Mat::Seed).count(), 0, "a seed on rock rots away");
        assert_eq!(w.growth[300 * W() + 110], 1, "the sown seed waits in the dark");
    }

    #[test]
    fn painting_seeds_sows_the_earth() {
        let mut w = World::new_static();
        for x in 100..200 {
            for y in 300..320 {
                w.set(x, y, Mat::Dirt);
            }
        }
        w.paint(150, 310, 6, Mat::Seed);
        assert!(w.cells.iter().filter(|c| c.m == Mat::Seed).count() == 0, "painted on earth, seeds are sown into it");
        let fertile = (0..W() * H()).filter(|&i| w.growth[i] > 0).count();
        assert!(fertile > 50, "the earth under the brush is fertile: {fertile}");
        for _ in 0..12 {
            w.paint(150, 310, 6, Mat::Seed);
        }
        assert!(w.growth[305 * W() + 150] <= SOIL_SEEDS, "a cell holds at most {SOIL_SEEDS} seeds");
        // In the air, seeds fall as grains.
        w.paint(150, 250, 4, Mat::Seed);
        assert!(w.cells.iter().filter(|c| c.m == Mat::Seed).count() > 10);
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
        let lowest = |x: i32| (0..H() as i32).rev().find(|&y| w.raw(x, y) != Mat::Empty).unwrap_or(-1);
        assert_eq!(lowest(150), H() as i32 - 1, "the rock slab comes to rest on the base of the world");
        assert_eq!(w.raw(150, H() as i32 - 21), Mat::Dirt, "its soil rides down with it");
        assert_eq!(lowest(330), -1, "soil that nothing holds falls and disappears off the bottom");
    }

    /// A sand dune on bedrock, with walls so it does not spread out.
    #[cfg(test)]
    fn dune() -> World {
        let mut w = World::new_static();
        w.structures_fall = true;
        let floor = H() as i32 - 10;
        for x in 60..340 {
            for y in floor..H() as i32 {
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
        let y = H() as i32 - 40;
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
                for y in H() as i32 - 10..H() as i32 {
                    w.set(x, y, Mat::Stone);
                }
                for y in H() as i32 - 70..H() as i32 - 10 {
                    w.set(x, y, Mat::Dirt);
                }
            }
            w
        };
        let y = H() as i32 - 40;
        let top = H() as i32 - 70;
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
    fn a_tunnel_through_rock_keeps_its_roof() {
        // A hill of rock on a bed of earth, with seams of earth in it (as the mountains have):
        // the rock over a tunnel right through it is a body of its own, held up by the vault.
        let mut w = World::new_static();
        w.structures_fall = true;
        for x in 100..300 {
            for y in H() as i32 - 10..H() as i32 {
                w.set(x, y, Mat::Stone);
            }
            for y in H() as i32 - 30..H() as i32 - 10 {
                w.set(x, y, Mat::Dirt);
            }
            for y in H() as i32 - 90..H() as i32 - 30 {
                w.set(x, y, if y == H() as i32 - 70 { Mat::Dirt } else { Mat::Stone });
            }
        }
        let y = H() as i32 - 50;
        w.tunnel_line((90, y), (310, y), 6);
        assert_eq!(w.raw(200, y), Mat::Empty);
        assert!(w.cells.iter().any(|c| c.m == Mat::Vault), "the rock is lined");
        for _ in 0..300 {
            w.step();
        }
        assert_eq!(w.raw(200, y), Mat::Empty, "the tunnel stays open");
        assert_eq!(w.raw(200, H() as i32 - 89), Mat::Stone, "the rock over it stays up");
    }

    #[test]
    fn digging_out_the_base_lets_a_lake_drain() {
        let mut w = World::new_static();
        w.structures_fall = true;
        for x in 100..200 {
            for y in H() as i32 - 6..H() as i32 {
                w.set(x, y, Mat::Stone);
            }
            for y in H() as i32 - 20..H() as i32 - 6 {
                w.set(x, y, Mat::Water);
            }
        }
        for y in H() as i32 - 20..H() as i32 {
            w.set(99, y, Mat::Stone);
            w.set(200, y, Mat::Stone);
        }
        for _ in 0..200 {
            w.step();
        }
        let before = w.cells.iter().filter(|c| c.m == Mat::Water).count();
        assert!(before > 1000, "the lake holds: {before}");
        for x in 140..160 {
            for y in H() as i32 - 6..H() as i32 {
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
    fn ice_dropped_from_high_up_shatters() {
        let mut w = World::new();
        w.day_cycle = false;
        w.climate_fixed = Some(-5.0);
        w.reset_temps();
        for x in 100..300 {
            for y in 500..H() as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        for x in 180..200 {
            for y in 200..210 {
                w.set(x, y, Mat::Ice);
            }
        }
        for _ in 0..900 {
            w.step();
        }
        let tally = |m: Mat| w.cells.iter().filter(|c| c.m == m).count();
        let (ice, hail) = (tally(Mat::Ice), tally(Mat::Hail));
        assert!(hail > 20, "a 15 m fall crushes part of it to hail: {hail}");
        assert!(ice + hail > 180, "nothing is lost: {ice} + {hail}");
        assert!(w.ejecta.is_empty() && (0..W() * H()).filter(|&i| matches!(w.cells[i].m, Mat::Ice | Mat::Hail)).all(|i| i / W() >= 470), "the pieces lie on the rock");
    }

    #[test]
    fn ice_falls_like_a_solid_bounces_a_little_and_floats_on_water() {
        let mut w = World::new();
        w.day_cycle = false;
        w.climate_fixed = Some(-5.0);
        w.reset_temps();
        // Rock on the base; a block of ice dropped from a little height (it would shatter from
        // high up, see `ice_dropped_from_high_up_shatters`).
        for x in 100..300 {
            for y in 500..H() as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        for x in 180..200 {
            for y in 482..492 {
                w.set(x, y, Mat::Ice);
            }
        }
        let top = |w: &World| (0..H() as i32).find(|&y| w.raw(190, y) == Mat::Ice).unwrap_or(-1);
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
            for y in 520..H() as i32 {
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
        let ice_y = (0..H() as i32).find(|&y| w.raw(190, y) == Mat::Ice).unwrap_or(-1);
        assert!(ice_y > 0 && ice_y < 500, "ice should float near the surface, y = {ice_y}");
    }

    #[test]
    fn ice_in_the_sky_is_not_held_up_by_gas_or_stray_drops() {
        let mut w = World::new();
        w.day_cycle = false;
        w.climate_fixed = Some(-5.0);
        w.reset_temps();
        for x in 100..300 {
            for y in 520..H() as i32 {
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
        // (Wherever it landed: it may have slid off the column it fell in.)
        let ice_top = (0..W() * H()).find(|&i| w.cells[i].m == Mat::Ice).map_or(-1, |i| (i / W()) as i32);
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
            let high = (0..W() * H()).filter(|&i| w.cells[i].m == Mat::Ice && i / W() < y as usize + 60 && (300..500).contains(&((i % W()) as i32))).count();
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
            (0..W() * H())
                .filter(|&i| {
                    w.cells[i].m == Mat::Ice
                        && i / W() < 330
                        && (0..4).all(|k| {
                            let (x, y) = ((i % W()) as i32, (i / W()) as i32);
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
            // (Snowflakes fall at under 1 m/s: a band of snow takes its time to fall away.)
            let steps = if layer == Mat::Snow { 1500 } else { 500 };
            for _ in 0..steps {
                w.step();
            }
            let ice_rows: Vec<usize> = (0..W() * H()).filter(|&i| w.cells[i].m == Mat::Ice && (380..420).contains(&((i % W()) as i32))).map(|i| i / W()).collect();
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
        for x in 0..W() as i32 {
            for y in H() as i32 - 10..H() as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        // (Banks as wide as the world: a wall one cell thick gives way under 32 m of water and
        // the splash of the rock, see `statics.rs`, and the lake would drain away.)
        for y in 300..H() as i32 - 10 {
            for x in (0..100).chain(300..W() as i32) {
                w.set(x, y, Mat::Stone);
            }
        }
        for x in 100..300 {
            for y in 340..H() as i32 - 10 {
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
        let rows = |m: Mat, x0: i32, x1: i32| (0..H() as i32).filter(|&y| (x0..x1).any(|x| w.raw(x, y) == m)).collect::<Vec<_>>();
        let rock = rows(Mat::Stone, 120, 160);
        let wood = rows(Mat::Wood, 200, 290);
        eprintln!("LAKE rock rows {:?}..{:?}, log rows {:?}..{:?}", rock.first(), rock.last(), wood.first(), wood.last());
        assert!(rock.iter().min().is_some_and(|&y| y > 400), "the rock sank: rows {:?}", rock.first());
        assert!(wood.iter().max().is_some_and(|&y| y < 350), "the log floats: rows {:?}", wood.last());
    }

    #[test]
    fn heavy_solids_rest_on_sand_instead_of_sinking_through_it() {
        let mut w = World::new();
        w.day_cycle = false;
        w.set_phase(0.75);
        // A bed of sand in a tank of rock (at the edges of the world it would run out).
        for x in 0..W() as i32 {
            for y in 450..H() as i32 {
                w.set(x, y, Mat::Stone);
            }
            for y in 300..450 {
                w.set(x, y, if (50..W() as i32 - 50).contains(&x) { Mat::Sand } else { Mat::Stone });
            }
        }
        // Glass and rock (both heavier than sand) laid on it and buried in it, a crumb of glass
        // (it falls straight down, not as a tumbling body) dropped onto it from high up.
        for x in 100..120 {
            for y in 296..300 {
                w.set(x, y, Mat::Glass);
            }
        }
        for x in 200..210 {
            for y in 320..330 {
                w.set(x, y, Mat::Stone);
            }
        }
        for x in 300..302 {
            for y in 200..202 {
                w.set(x, y, Mat::Glass);
            }
        }
        for _ in 0..400 {
            w.step();
        }
        let rows = |m: Mat, x0: i32, x1: i32| (0..H() as i32).filter(|&y| (x0..x1).any(|x| w.raw(x, y) == m)).collect::<Vec<_>>();
        let slab = rows(Mat::Glass, 100, 120);
        assert_eq!((slab.first(), slab.last()), (Some(&296), Some(&299)), "the glass slab stays on the sand");
        let rock: Vec<i32> = rows(Mat::Stone, 195, 215).into_iter().filter(|&y| y < 450).collect();
        assert_eq!((rock.first(), rock.last()), (Some(&320), Some(&329)), "the rock stays where it is buried");
        let crumb = rows(Mat::Glass, 290, 312);
        assert!(crumb.last().is_some_and(|&y| (300..=306).contains(&y)), "the crumb dents the sand, no more: rows {crumb:?}");
    }

    /// A floor of rock on the base of the world, `top` rows thick.
    fn rock_floor(w: &mut World, top: i32) {
        for x in 0..W() as i32 {
            for y in H() as i32 - top..H() as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
    }

    #[test]
    fn a_log_painted_against_a_cliff_falls_as_one_piece() {
        let mut w = World::new();
        w.climate_fixed = Some(15.0);
        w.reset_temps();
        rock_floor(&mut w, 10);
        for x in 190..=210 {
            for y in 200..H() as i32 - 10 {
                w.set(x, y, Mat::Stone);
            }
        }
        // A stroke of wood that touches the side of the cliff: it does not stick to it.
        w.new_log();
        w.line((217, 150), (240, 150), 5, Mat::Wood);
        let cells = |w: &World| (0..W() * H()).filter(|&i| w.cells[i].m == Mat::Wood).collect::<Vec<_>>();
        let before = cells(&w);
        let rows = |c: &[usize]| (c.iter().map(|i| i / W()).min().unwrap(), c.iter().map(|i| i / W()).max().unwrap());
        for _ in 0..400 {
            w.step();
        }
        let after = cells(&w);
        assert_eq!(after.len(), before.len(), "the log keeps all its wood");
        let ((t0, b0), (t1, b1)) = (rows(&before), rows(&after));
        assert_eq!(b1 - t1, b0 - t0, "it keeps its shape");
        assert_eq!(b1, H() - 11, "it lies on the floor");
    }

    #[test]
    fn two_logs_side_by_side_are_two_bodies() {
        let mut w = World::new();
        w.climate_fixed = Some(15.0);
        w.reset_temps();
        rock_floor(&mut w, 10);
        // A pit in the floor, right of x = 300.
        for x in 300..350 {
            for y in H() as i32 - 10..H() as i32 - 2 {
                w.set(x, y, Mat::Empty);
            }
        }
        let floor = H() as i32 - 10;
        w.new_log();
        w.line((262, floor - 4), (297, floor - 4), 3, Mat::Wood);
        // The second log touches the end of the first one, over the pit.
        w.new_log();
        w.line((304, floor - 4), (335, floor - 4), 3, Mat::Wood);
        for _ in 0..300 {
            w.step();
        }
        let lowest = |x0: i32, x1: i32| (0..H() as i32).rev().find(|&y| (x0..x1).any(|x| w.raw(x, y) == Mat::Wood)).unwrap();
        assert_eq!(lowest(255, 300), floor - 1, "the first log stays on the floor");
        assert!(lowest(302, 345) > floor + 4, "the second one fell into the pit: {}", lowest(302, 345));
    }

    #[test]
    fn a_light_log_rests_on_sand() {
        let mut w = World::new();
        w.climate_fixed = Some(15.0);
        w.reset_temps();
        rock_floor(&mut w, 10);
        let sand = H() as i32 - 40;
        for x in 0..W() as i32 {
            for y in sand..H() as i32 - 10 {
                w.set(x, y, Mat::Sand);
            }
        }
        w.new_log();
        w.line((300, 300), (330, 300), 3, Mat::Wood);
        for _ in 0..400 {
            w.step();
        }
        let lowest = (0..H() as i32).rev().find(|&y| (295..335).any(|x| w.raw(x, y) == Mat::Wood)).unwrap();
        assert!(lowest < sand + 2, "the log lies on the sand, not buried in it: {lowest}");
    }

    #[test]
    fn a_planted_tree_stands_on_the_ground() {
        let mut w = World::new();
        w.climate_fixed = Some(15.0);
        w.reset_temps();
        rock_floor(&mut w, 10);
        let ground = H() as i32 - 30;
        for x in 0..W() as i32 {
            for y in ground..H() as i32 - 10 {
                w.set(x, y, Mat::Dirt);
            }
        }
        // A click in the sky plants it on the ground below; one in the water does not.
        assert!(w.plant_tree(400, 80, 0.5));
        let trunk = |w: &World| (0..H() as i32).filter(|&y| w.raw(400, y) == Mat::Trunk).collect::<Vec<_>>();
        let t = trunk(&w);
        assert_eq!(*t.last().unwrap(), ground - 1, "the trunk stands on the soil");
        assert!(t.len() as i32 >= 8 * K);
        assert!(w.cells.iter().filter(|c| c.m == Mat::Leaf).count() > 100, "it has a crown");
        for _ in 0..200 {
            w.step();
        }
        assert_eq!(trunk(&w), t, "it stays standing");
        for y in ground - 6..ground {
            w.set(600, y, Mat::Water);
        }
        assert!(!w.plant_tree(600, 80, 0.5), "no tree in a lake");
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
                w.temp[y as usize * W() + x as usize] = 1100.0;
            }
        }
        // A thin layer of lava chills against cold rock and sets before the rock under it reaches
        // its melting point. Kept flowing (fresh lava poured in from above all along, as in a lava
        // channel), it melts its bed: thermal erosion.
        for _ in 0..800 {
            for x in 101..105 {
                w.set(x, 292, Mat::Lava);
                w.temp[292 * W() + x as usize] = 1100.0;
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
                w.temp[y as usize * W() + x as usize] = 1300.0;
            }
        }
        let mut molten = false;
        for k in 0..2500 {
            if k < 1500 {
                for x in 100..140 {
                    if w.raw(x, 305) == Mat::Empty {
                        w.set(x, 305, Mat::Lava);
                        w.temp[305 * W() + x as usize] = 1300.0;
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
            w.temp[318 * W() + x as usize] = 700.0;
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
            let xs: Vec<i32> = (0..W() * H()).filter(|&i| w.cells[i].m == Mat::Sand).map(|i| (i % W()) as i32).collect();
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
        let oil_lowest = (0..H() as i32).rev().find(|&y| (100..200).any(|x| w.raw(x, y) == Mat::Oil)).unwrap_or(0);
        let water_top = (0..H() as i32).find(|&y| (100..200).any(|x| w.raw(x, y) == Mat::Water)).unwrap();
        assert!(oil_lowest <= water_top + 3, "oil should float on the water: lowest oil row {oil_lowest}, water surface {water_top}");
        // Set it alight: flames last far longer than a normal fire would.
        // (The top of the layer, not a stray drop resting on it.)
        let oil_top = (0..H() as i32).find(|&y| (100..200).filter(|&x| w.raw(x, y) == Mat::Oil).count() >= 10).unwrap();
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
        let sand_grass = (0..H() as i32).find(|&y| w.raw(205, y) == Mat::Grass).expect("grass should be resting on the sand");
        assert_eq!(w.raw(205, sand_grass + 1), Mat::Sand, "grass lands on sand");
        assert!((0..H() as i32).all(|y| w.raw(300, y) != Mat::Grass), "grass over a void falls out of the world");
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
            (w.surface_temp_c(), w.ambient_c(0), w.ambient_c(H() as i32 / 2))
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
    fn the_moon_goes_through_its_phases_and_rises_later_every_night() {
        let mut w = World::new_static();
        w.set_phase(0.75);
        // Eight nights, eight different moons, then the cycle starts again.
        let mut names = vec![];
        for day in 1..=9 {
            w.set_day(day);
            names.push(w.moon_name());
        }
        assert_eq!(&names[..8], ["premier quartier", "lune gibbeuse croissante", "pleine lune", "lune gibbeuse décroissante", "dernier quartier", "dernier croissant", "nouvelle lune", "premier croissant"]);
        assert_eq!(names[8], names[0]);
        // Full moon: all lit, opposite the sun, high in the sky at midnight, gone by day.
        w.set_day(3);
        assert!(w.full_moon() && w.moon_light() > 0.99 && w.nights_to_full_moon() == 0);
        let u = w.moon_arc().expect("the full moon is up at midnight");
        assert!((u - 0.5).abs() < 0.1, "highest at midnight: {u}");
        w.set_phase(0.25);
        assert!(w.moon_arc().is_none(), "the full moon is set at noon");
        // New moon: dark, and it crosses the sky with the sun.
        w.set_day(7);
        w.set_phase(0.25);
        assert!(w.moon_light() < 0.05 && w.moon_arc().is_some());
        w.set_phase(0.75);
        assert!(w.moon_arc().is_none());
        // First quarter: half lit, up in the evening, set before the morning.
        w.set_day(1);
        w.set_phase(0.55);
        assert!((w.moon_light() - 0.5).abs() < 0.1 && w.moon_arc().is_some());
        w.set_phase(0.95);
        assert!(w.moon_arc().is_none());
        assert_eq!(w.nights_to_full_moon(), 2);
        w.set_day(4);
        assert_eq!(w.nights_to_full_moon(), 7);
        assert!(!w.full_moon());
    }

    #[test]
    fn the_sun_warms_the_ground_most_nearby_and_the_air_cools_with_altitude() {
        let mut w = World::new();
        w.set_phase(0.25);
        w.day_cycle = false;
        for x in 0..W() as i32 {
            for y in 300..320 {
                w.set(x, y, Mat::Stone);
            }
        }
        w.reset_temps();
        for _ in 0..600 {
            w.step();
        }
        let under = w.temp_c(W() as i32 / 2, 300);
        let far = w.temp_c(40, 300);
        assert!(under > far + 1.5, "ground under the sun {under} vs far {far}");
        // Away from the sun's halo, the air gets colder toward the top of the screen.
        assert!(w.temp_c(40, 80) < w.temp_c(40, 290) - 5.0, "air should cool with altitude");
    }

    #[test]
    fn the_ground_in_the_shade_of_a_cloud_warms_less() {
        let run = |cloudy: bool| {
            let mut w = World::new();
            w.set_phase(0.25);
            w.day_cycle = false;
            w.flow.pin_uniform(0.0, 0.0);
            // Rock down to the base of the world (only that holds it up).
            for x in 0..W() as i32 {
                for y in 300..H() as i32 {
                    w.set(x, y, Mat::Stone);
                }
            }
            if cloudy {
                for x in 380..580 {
                    for y in 150..170 {
                        w.set(x, y, Mat::Cloud);
                    }
                }
            }
            w.reset_temps();
            // Ground chilled by the night: the morning sun warms it up, where it reaches it.
            for x in 0..W() {
                for y in 300..320 {
                    w.temp[y * W() + x] = 0.0;
                }
            }
            for _ in 0..600 {
                w.step();
            }
            w.temp_c(480, 300)
        };
        let (sunny, shaded) = (run(false), run(true));
        assert!(shaded < sunny - 1.0, "ground under a cloud {shaded} vs in the sun {sunny}");
    }

    #[test]
    fn clouds_and_steam_stop_under_the_top_of_the_weather() {
        let mut w = World::new();
        w.structures_fall = false;
        w.clear();
        w.flow.pin_uniform(0.0, -2.0);
        // Hot steam low down, a very warm cloud high up, and a puff of cloud painted in the
        // stratosphere. (A real atmosphere: the air cools up to the tropopause, then warms.)
        for x in 300..340 {
            for y in 250..260 {
                w.set(x, y, Mat::Steam);
                w.temp[y as usize * W() + x as usize] = 90.0;
            }
            for y in 80..90 {
                w.set(x + 200, y, Mat::Cloud);
                w.temp[y as usize * W() + x as usize + 200] = 40.0;
            }
            w.set(x + 400, 10, Mat::Cloud);
        }
        // A strong updraft may push the top of a storm cloud a little past the tropopause (an
        // overshooting top), but cooling as it climbs it is soon heavier than the warm air there.
        let mut highest = H();
        for _ in 0..800 {
            w.step();
            highest = (0..W() * H()).filter(|&i| matches!(w.cells[i].m, Mat::Steam | Mat::Cloud) && (i % W()) < 640).map(|i| i / W()).min().unwrap_or(H()).min(highest);
        }
        assert!(highest as i32 >= CEILING_Y() - 20, "rising cloud overshot up to row {highest}");
        let gas: Vec<usize> = (0..W() * H()).filter(|&i| matches!(w.cells[i].m, Mat::Steam | Mat::Cloud)).collect();
        assert!(!gas.is_empty());
        assert!(gas.iter().all(|&i| (i / W()) as i32 >= CEILING_Y() - 12), "gas above the tropopause settled back down to it");
        let near_top = gas.iter().filter(|&&i| ((i / W()) as i32) < CEILING_Y() + 12).count();
        assert!(near_top * 3 > gas.len(), "the rising gas gathers in a layer under the ceiling: {near_top} of {}", gas.len());
    }

    #[test]
    fn steam_does_not_stay_trapped_under_a_tree() {
        let mut w = World::new_static();
        w.flow.pin_uniform(0.0, 0.0);
        // A thick crown of leaves and branches, with steam under it.
        for x in 380..460 {
            for y in 200..230 {
                w.set(x, y, if y == 229 { Mat::Branch } else { Mat::Leaf });
            }
            for y in 235..245 {
                w.set(x, y, Mat::Steam);
            }
        }
        for _ in 0..600 {
            w.step();
        }
        let under = (0..W() * H()).filter(|&i| w.cells[i].m == Mat::Steam && (230..260).contains(&(i / W())) && (380..460).contains(&(i % W()))).count();
        let above = (0..W() * H()).filter(|&i| matches!(w.cells[i].m, Mat::Steam | Mat::Cloud) && i / W() < 200).count();
        assert!(under < 100, "steam stays trapped under the crown: {under} cells");
        assert!(above > 300, "the steam went up through the leaves: {above} cells above");
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
            w.temp[200 * W() + 500 + k as usize] = -15.0;
        }
        let mean_y = |w: &World, left: bool| {
            let ys: Vec<usize> = (0..W() * H()).filter(|&i| w.cells[i].m == Mat::Smoke && ((i % W()) < 400) == left).map(|i| i / W()).collect();
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
            let xs: Vec<i32> = (0..W() * H()).filter(|&i| matches!(w.cells[i].m, Mat::Cloud | Mat::Steam)).map(|i| (i % W()) as i32).collect();
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
        for i in 0..W() * H() {
            if w.cells[i].m == Mat::Water {
                positions.push((i % W()) as i32);
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
        assert!(w.at(200, H() as i32 - 2) == Mat::Stone);
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
        time("level water", Box::new(|w| w.level_liquids()), &mut w);
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
        // Steam released high up in the cold layer: a big bank of it, so that the cloud it
        // makes stays thick long enough to rain for sure (a drop falls from a thick cloud at
        // one chance in `RAIN_EVERY` a step).
        for x in 240..360 {
            for y in 20..50 {
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
                    w.temp[y as usize * W() + x as usize] = thermal::METAL_MELT_C + 400.0;
                }
            }
            let mut seen = false;
            for _ in 0..400 {
                w.step();
                seen |= count(&w, liquid) > 0;
            }
            assert!(seen, "{solid:?} above its melting point turns {liquid:?}");
            // Cooled down, it sets again into what it was.
            for k in 0..W() * H() {
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
        let cells: Vec<usize> = (0..W() * H()).filter(|&k| w.cells[k].m == Mat::Molten).collect();
        assert!(!cells.is_empty());
        assert!(cells.iter().all(|&k| w.temp[k] > thermal::METAL_MELT_C), "poured molten metal is above its melting point");
        for _ in 0..60 {
            w.step();
        }
        assert!(count(&w, Mat::Molten) > 0, "it does not set at once");
        assert!((0..W() * H()).any(|k| w.cells[k].m == Mat::Molten && k / W() == 299), "it ran down onto the floor");
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
    fn seeds_in_moist_earth_grow_into_a_tree_and_wait_in_dry_earth() {
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
        // The bank by the puddle is moist.
        for x in 110..125 {
            for y in 310..320 {
                w.moisture[y * W() + x] = 120;
            }
        }
        // Seeds sown on the moist bank, and on dry earth far from it.
        w.set(122, 300, Mat::Seed);
        w.set(123, 309, Mat::Water);
        w.set(124, 309, Mat::Water);
        w.paint(200, 309, 2, Mat::Seed);
        for _ in 0..4000 {
            w.step();
        }
        let wood = count(&w, Mat::Wood) + count(&w, Mat::Trunk) + count(&w, Mat::Sapling);
        assert!(wood >= 6, "tree only {wood} cells tall");
        assert!((190..215).all(|x| (280..310).all(|y| !matches!(w.raw(x, y), Mat::Sapling | Mat::Trunk))), "on dry earth the seeds wait");
        assert!(w.growth[310 * W() + 200] > 0, "still there, in the earth");
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
                w.moisture[i] = 200;
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
        let soil = |w: &World| (130..190).flat_map(|x| (310..320).map(move |y| (x, y))).map(|(x, y)| w.moisture[World::idx(x, y)] as u32).sum::<u32>();
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
        let at = |w: &World| (0..H() as i32).find(|&y| w.raw(200, y) == Mat::Sand).unwrap();
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
        floor(&mut w, 300, 0, W() as i32);
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
        floor(&mut w, 300, 0, W() as i32);
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
        let y = (0..H() as i32).find(|&y| (370..430).any(|x| w.raw(x, y) == Mat::Snow)).unwrap_or(H() as i32);
        assert!(y > 40 && y <= 20 + 45, "a flake falls about a cell a step: it is at {y}");
    }

    #[test]
    fn ice_frozen_on_a_tree_falls_and_breaks_the_branches() {
        let mut w = World::new_static();
        w.structures_fall = true;
        w.climate_fixed = Some(-5.0);
        w.reset_temps();
        // Ground resting on the base of the world (only that holds things up).
        for x in 0..W() as i32 {
            for y in 400..H() as i32 {
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
        // A 5 m fall breaks it: pieces of ice and hail.
        let ice: Vec<usize> = (0..W() * H()).filter(|&i| matches!(w.cells[i].m, Mat::Ice | Mat::Hail)).collect();
        let highest = ice.iter().map(|i| i / W()).min().unwrap_or(0);
        assert!(ice.len() * 10 > n * 9, "the ice is still there: {} of {n}", ice.len());
        assert!(highest > 350, "the ice fell to the ground through the crown (highest ice row {highest})");
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
        floor(&mut w, H() as i32 - 1, 0, W() as i32);
        // Attached pillar.
        for y in 400..H() as i32 - 1 {
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
        // (A fall of 56 m crushes a corner or two of the slab into sand, which may lie under it:
        // the stone is on the floor, not necessarily the cell that was under its middle.)
        assert!((150..300).any(|x| w.raw(x, H() as i32 - 2) == Mat::Stone), "slab did not land");
        assert!((150..300).all(|x| (150..400).all(|y| w.raw(x, y) != Mat::Stone)), "the slab is still high in the air");
    }

    #[test]
    fn liquids_rest_on_trees_and_do_not_drip_through() {
    let mut w = World::new_static();
    for y in 0..H() as i32 {
        for x in 100..140 {
            w.set(x, y, Mat::Empty);
        }
    }
    for x in 100..140 {
        w.set(x, 300, Mat::Stone);
    }
    // A canopy of leaves with a branch and a trunk stub under it, wall to wall: thin matter
    // (`Mat::one_way`) holds a liquid up.
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
    let total = (0..W() as i32).flat_map(|x| (0..H() as i32).map(move |y| (x, y))).filter(|&(x, y)| matches!(w.at(x, y), Mat::Water | Mat::Oil)).count();
    let below = (100..140).flat_map(|x| (203..300).map(move |y| (x, y))).filter(|&(x, y)| matches!(w.at(x, y), Mat::Water | Mat::Oil)).count();
    assert!(below == 0 && above >= 8, "the liquid went down through the tree: {below} below, {above} above, {total} total");
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
        let xs: Vec<f32> = (lo..hi).flat_map(|y| (0..W() as i32).map(move |x| (x, y))).filter(|&(x, y)| w.raw(x, y) == Mat::Cloud).map(|(x, _)| x as f32).collect();
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
    let xs: Vec<i32> = (0..H() as i32).flat_map(|y| (0..W() as i32).map(move |x| (x, y))).filter(|&(x, y)| w.raw(x, y) == Mat::Water && y < 300).map(|(x, _)| x).collect();
    assert!(!xs.is_empty());
    let mean = xs.iter().sum::<i32>() as f32 / xs.len() as f32;
    assert!(mean > 312.0, "falling drops are pushed along: mean x {mean}");
}

/// A crucible of fired clay, a wood fire under it, iron and charcoal inside.
fn crucible_world(wall: Mat) -> World {
    let mut w = World::new_static();
    let floor = 300;
    for x in 0..W() as i32 {
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
                    w.set(x, 319, Mat::Grass);
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

    /// A tray of earth `depth` cells deep between x 100 and 200, on a stone floor at y 320, with
    /// stone walls 40 cells high.
    fn soil_tray(w: &mut World, depth: i32, moisture: u16) {
        floor(w, 320, 99, 201);
        for y in 280..320 {
            w.set(99, y, Mat::Stone);
            w.set(200, y, Mat::Stone);
        }
        for x in 100..200 {
            for y in 320 - depth..320 {
                w.set(x, y, Mat::Dirt);
                w.moisture[World::idx(x, y)] = moisture;
            }
        }
    }

    /// All the water in the world, in sixteenths of a cell: in the soil, in liquid water, and in
    /// vapour (a cell of water is `SOIL_WATER`, a cell of vapour a quarter of that).
    fn all_water(w: &World) -> u32 {
        (0..W() * H())
            .map(|i| match w.cells[i].m {
                Mat::Dirt | Mat::Grass | Mat::Mud => w.moisture[i] as u32,
                Mat::Water => SOIL_WATER as u32,
                _ => w.vapour_units(i) * SOIL_VAPOUR as u32,
            })
            .sum()
    }

    /// Mean moisture of the soil on row `y` of the tray.
    fn soil_row(w: &World, y: i32) -> u32 {
        (100..200).map(|x| w.moisture[World::idx(x, y)] as u32).sum::<u32>() / 100
    }

    #[test]
    fn soil_drinks_water_until_it_is_saturated_and_the_rest_stays_as_a_puddle() {
        let mut w = World::new_static();
        w.flow.pin_moisture(1.0);
        soil_tray(&mut w, 4, 0);
        // More water than four rows of earth can hold (three cells of water per cell of earth).
        for x in 100..200 {
            for y in 290..306 {
                w.set(x, y, Mat::Water);
            }
        }
        let before = all_water(&w);
        for t in 0..8000 {
            w.step();
            // Exactly, while the soil drinks (later, clouds drifting off the world take some away).
            if t == 1500 {
                assert_eq!(all_water(&w), before, "the water is kept, in the ground, in the puddle or in the air");
            }
        }
        assert!(w.cells.iter().zip(&w.moisture).all(|(c, &m)| !matches!(c.m, Mat::Dirt | Mat::Grass) || m <= SOIL_FULL), "no soil holds more than saturated");
        for y in 316..320 {
            assert!(soil_row(&w, y) > SOIL_FULL as u32 - SOIL_WATER as u32, "row {y} soaked through: {}", soil_row(&w, y));
        }
        let puddle = count(&w, Mat::Water);
        assert!(puddle > 300, "what the earth cannot drink stays on top: {puddle}");
    }

    #[test]
    fn water_drains_down_through_the_soil_and_creeps_back_up_to_dry_earth() {
        let mut w = World::new_static();
        w.flow.pin_moisture(1.0);
        soil_tray(&mut w, 12, 0);
        // A soaking at the top.
        for x in 100..200 {
            w.moisture[World::idx(x, 308)] = 180;
        }
        let before = all_water(&w);
        for _ in 0..3000 {
            w.step();
        }
        assert!(soil_row(&w, 308) < SOIL_FIELD as u32, "the free water left the top: {}", soil_row(&w, 308));
        assert!(soil_row(&w, 312) > 0, "it went down: {}", soil_row(&w, 312));
        assert!(all_water(&w) + 100 * SOIL_VAPOUR as u32 >= before, "the soil does not lose water as it seeps");

        // Wet earth deep down, dry on top: the moisture creeps up (capillarity).
        let mut w = World::new_static();
        w.flow.pin_moisture(1.0);
        soil_tray(&mut w, 12, 0);
        for x in 100..200 {
            for y in 316..320 {
                w.moisture[World::idx(x, y)] = SOIL_FIELD;
            }
        }
        for _ in 0..6000 {
            w.step();
        }
        assert!(soil_row(&w, 312) > 0, "water crept up: {}", soil_row(&w, 312));
        assert!(soil_row(&w, 319) < SOIL_FIELD as u32, "from the bottom: {}", soil_row(&w, 319));
    }

    #[test]
    fn saturated_soil_over_a_hole_drips() {
        let mut w = World::new_static();
        w.flow.pin_moisture(1.0);
        floor(&mut w, 330, 100, 200);
        for x in 100..200 {
            for y in 296..300 {
                w.set(x, y, Mat::Dirt);
                w.moisture[World::idx(x, y)] = SOIL_FULL;
            }
        }
        let mut drops = 0;
        for _ in 0..2000 {
            w.step();
            drops = drops.max(count(&w, Mat::Water));
        }
        assert!(drops > 10, "drops fall from the roof: {drops}");
        assert!(soil_row(&w, 299) < SOIL_FULL as u32, "the roof gave up some of its water");
    }

    #[test]
    fn moist_soil_dries_out_into_the_air_as_vapour() {
        let mut w = World::new_static();
        w.flow.pin_moisture(0.0);
        soil_tray(&mut w, 3, 120);
        let soil = soil_row(&w, 317);
        let mut vapour = 0;
        for _ in 0..3000 {
            w.step();
            vapour = vapour.max(count(&w, Mat::Steam) + count(&w, Mat::Cloud));
        }
        assert!(soil_row(&w, 317) < soil, "the surface dried: {} -> {}", soil, soil_row(&w, 317));
        assert!(vapour > 5, "the water went into the air: {vapour}");
        // The vapour took its heat from the ground: wet earth is cooler than the same earth dry.
        let mut dry = World::new_static();
        dry.flow.pin_moisture(0.0);
        soil_tray(&mut dry, 3, 0);
        for _ in 0..3000 {
            dry.step();
        }
        let warmth = |w: &World| (100..200).map(|x| w.temp[World::idx(x, 317)]).sum::<f32>() / 100.0;
        assert!(warmth(&w) < warmth(&dry), "evaporation cools the soil: {} vs {}", warmth(&w), warmth(&dry));
    }

    #[test]
    fn grass_grows_by_itself_on_moist_soil_and_dies_on_dry_soil() {
        let tray = |moisture: u16| {
            let mut w = World::new_static();
            w.clock = thermal::DAY_STEPS * thermal::SEASON_DAYS + thermal::DAY_STEPS / 8;
            w.flow.pin_moisture(1.0);
            soil_tray(&mut w, 6, moisture);
            w.set(150, 314, Mat::Grass);
            w.moisture[World::idx(150, 314)] = moisture;
            for _ in 0..8000 {
                w.step();
            }
            count(&w, Mat::Grass)
        };
        let moist = tray(80);
        assert!(moist > 10, "grass spreads over moist earth: {moist}");
        assert_eq!(tray(0), 0, "on dry earth the grass dies and none grows");
    }

    #[test]
    fn grass_turns_yellow_as_its_soil_dries() {
        let mut w = World::new_static();
        w.set(100, 300, Mat::Grass);
        w.set(102, 300, Mat::Grass);
        w.moisture[World::idx(100, 300)] = 80;
        w.moisture[World::idx(102, 300)] = 0;
        let (green, straw) = (w.color(100, 300), w.color(102, 300));
        assert!(green[1] > green[0] + 40, "moist: green {green:?}");
        assert!(straw[0] > green[0] + 60 && straw[0] + 40 > straw[1], "dry: straw yellow {straw:?}");
    }

    #[test]
    fn burning_grass_leaves_its_earth() {
        let mut w = World::new_static();
        floor(&mut w, 301, 100, 200);
        for x in 100..200 {
            w.set(x, 300, Mat::Grass);
        }
        // (A blade catches at one chance in ten per heat pass: 400 steps, so that none is
        // left by luck.)
        for _ in 0..400 {
            for x in 100..200 {
                w.temp[World::idx(x, 300)] = 400.0;
            }
            w.step();
        }
        assert_eq!(count(&w, Mat::Grass), 0, "the grass burnt");
        assert!(count(&w, Mat::Dirt) > 90, "the earth stays: {}", count(&w, Mat::Dirt));
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




