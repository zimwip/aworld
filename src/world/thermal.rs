//! Temperature, sun, day and night, wind. Every cell (air included) has a
//! temperature in °C; heat flows between neighbours, materials react when it
//! crosses a threshold, and the sun and the night sky push the whole world.

use super::*;

/// Simulation steps in one full day.
pub const DAY_STEPS: u32 = 21_600;
/// Days per season; a year is four seasons.
pub const SEASON_DAYS: u32 = 2;
/// Row considered "ground level" for the air temperature profile.
const GROUND_Y: f32 = 360.0;

pub const FIRE_C: f32 = 650.0;
pub const EMBER_C: f32 = 500.0;
pub const LAVA_C: f32 = 1100.0;
pub const BOIL_C: f32 = 100.0;
pub const FREEZE_C: f32 = -6.0;
pub const MELT_C: f32 = 3.0;
/// Lava turns to rock once it has cooled below this.
pub const LAVA_SOLID_C: f32 = 390.0;
pub const ROCK_MELT_C: f32 = 540.0;
pub const GLASS_C: f32 = 600.0;
/// Metal melts only in fresh lava and sets again once it cools below this.
pub const METAL_MELT_C: f32 = 1000.0;
pub const METAL_SOLID_C: f32 = 850.0;
pub const STEEL_C: f32 = 450.0;
pub const SMELT_C: f32 = 400.0;
pub const WOOD_IGNITE_C: f32 = 300.0;
pub const POWDER_IGNITE_C: f32 = 210.0;
/// A glass cell whose neighbours differ by more than this cracks.
pub const SHOCK_C: f32 = 320.0;
/// Oil catches fire by itself at this temperature.
pub const OIL_IGNITE_C: f32 = 120.0;
/// The thermal pass runs every this many steps, scaled up to keep the same speed.
pub const THERMAL_EVERY: u32 = 3;
const TS: f32 = THERMAL_EVERY as f32 / 2.0;

/// (conductivity, heat capacity) per material. High capacity means slow to
/// heat up and slow to cool down: lava keeps its heat for a long time.
fn props(m: Mat) -> (f32, f32) {
    match m {
        Mat::Empty => (0.12, 1.0),
        Mat::Oil => (0.1, 2.0),
        Mat::Steam | Mat::Cloud | Mat::Smoke => (0.12, 1.5),
        Mat::Water => (0.25, 3.0),
        Mat::Ice => (0.15, 2.0),
        Mat::Snow | Mat::Hail => (0.05, 1.5),
        Mat::Sand | Mat::Dirt | Mat::Mud | Mat::Grass => (0.06, 1.8),
        Mat::Wood | Mat::Trunk | Mat::Leaf | Mat::Seed | Mat::Sapling | Mat::Branch | Mat::DeadLeaf => (0.04, 1.4),
        Mat::Stone | Mat::Brick => (0.05, 6.0),
        Mat::Iron | Mat::Steel => (0.5, 2.5),
        Mat::Glass | Mat::Shard => (0.08, 3.0),
        Mat::Lava => (0.2, 400.0),
        Mat::Molten => (0.5, 300.0),
        Mat::Fire => (0.4, 1.0),
        Mat::Ember => (0.25, 1.0),
        Mat::Gunpowder | Mat::Ash | Mat::Rust => (0.04, 1.2),
    }
}

struct Tables {
    k: [f32; NMAT],
    inv_c: [f32; NMAT],
    reactive: [bool; NMAT],
}

fn tables() -> &'static Tables {
    static T: std::sync::OnceLock<Tables> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = Tables { k: [0.1; NMAT], inv_c: [1.0; NMAT], reactive: [false; NMAT] };
        for m in Mat::ALL {
            let (k, c) = props(m);
            t.k[m as usize] = k;
            t.inv_c[m as usize] = 1.0 / c;
            t.reactive[m as usize] = matches!(
                m,
                Mat::Water | Mat::Ice | Mat::Snow | Mat::Hail | Mat::Steam | Mat::Lava | Mat::Molten | Mat::Stone | Mat::Sand
                    | Mat::Shard | Mat::Glass | Mat::Rust | Mat::Iron | Mat::Steel | Mat::Wood | Mat::Trunk | Mat::Leaf | Mat::Branch | Mat::DeadLeaf
                    | Mat::Grass | Mat::Seed | Mat::Sapling | Mat::Gunpowder | Mat::Oil
            );
        }
        t
    })
}

/// Where a material changes state. Every solid/liquid/gas change goes through
/// this one table, the same way for all of them: above the first threshold it
/// becomes the first material, below the second it becomes the second.
struct Phase {
    up: Option<(f32, Mat, u32)>,
    down: Option<(f32, Mat, u32)>,
}

fn phase(m: Mat) -> Phase {
    let none = Phase { up: None, down: None };
    match m {
        // ice <-> water <-> steam
        Mat::Water => Phase { up: Some((BOIL_C, Mat::Steam, 1)), down: Some((FREEZE_C, Mat::Ice, 1)) },
        Mat::Ice | Mat::Snow | Mat::Hail => Phase { up: Some((MELT_C, Mat::Water, 1)), down: None },
        // rock <-> lava
        Mat::Stone => Phase { up: Some((ROCK_MELT_C, Mat::Lava, 30)), down: None },
        Mat::Lava => Phase { up: None, down: Some((LAVA_SOLID_C, Mat::Stone, 20)) },
        // iron and steel <-> molten metal (only fresh lava is hot enough)
        Mat::Iron | Mat::Steel => Phase { up: Some((METAL_MELT_C, Mat::Molten, 40)), down: None },
        Mat::Molten => Phase { up: None, down: Some((METAL_SOLID_C, Mat::Iron, 20)) },
        // sand fuses into glass
        Mat::Sand | Mat::Shard => Phase { up: Some((GLASS_C, Mat::Glass, 15)), down: None },
        _ => none,
    }
}

/// How much of the sun's light a material soaks up.
fn absorb(m: Mat) -> f32 {
    match m {
        Mat::Water => 0.6,
        Mat::Snow | Mat::Ice => 0.2,
        Mat::Sand | Mat::Glass => 0.8,
        Mat::Iron | Mat::Steel => 0.9,
        _ => 1.0,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Season {
    Spring,
    Summer,
    Autumn,
    Winter,
}

impl Season {
    pub const ALL: [Season; 4] = [Season::Spring, Season::Summer, Season::Autumn, Season::Winter];

    pub fn name(self) -> &'static str {
        match self {
            Season::Spring => "Printemps",
            Season::Summer => "Été",
            Season::Autumn => "Automne",
            Season::Winter => "Hiver",
        }
    }

    pub fn index(self) -> u32 {
        Self::ALL.iter().position(|s| *s == self).unwrap() as u32
    }
}

impl World {
    // ----- seasons ----------------------------------------------------------

    fn year_steps() -> u32 {
        DAY_STEPS * SEASON_DAYS * 4
    }

    /// 0.0 at the start of spring, up to 1.0 at the end of winter.
    pub fn year_phase(&self) -> f32 {
        (self.clock % Self::year_steps()) as f32 / Self::year_steps() as f32
    }

    pub fn season(&self) -> Season {
        Season::ALL[((self.year_phase() * 4.0) as usize).min(3)]
    }

    /// How far we are into the current season, 0..1.
    pub fn season_progress(&self) -> f32 {
        (self.year_phase() * 4.0).fract()
    }

    /// Jump to the start of a season (morning of its first day).
    pub fn set_season(&mut self, s: Season) {
        let year = self.clock / Self::year_steps();
        self.clock = year * Self::year_steps() + s.index() * SEASON_DAYS * DAY_STEPS + DAY_STEPS / 5;
        self.update_season_cache();
    }

    /// Seasonal shift of the air temperature: warmest mid-summer, coldest mid-winter.
    fn season_wave(&self) -> f32 {
        ((self.year_phase() - 0.125) * std::f32::consts::TAU).sin()
    }

    /// Warmest the ground gets by day: 10 °C mid-winter up to 40 °C mid-summer.
    pub fn day_peak_c(&self) -> f32 {
        25.0 + 15.0 * self.season_wave()
    }

    /// Coldest the ground gets by night: -10 °C mid-winter, -5 °C mid-summer.
    pub fn night_low_c(&self) -> f32 {
        -7.5 + 2.5 * self.season_wave()
    }

    /// Refreshes the leaf and grass colours for the current season.
    pub(super) fn update_season_cache(&mut self) {
        self.revision += 1;
        let p = self.season_progress();
        let mix = |a: (i32, i32, i32), b: (i32, i32, i32), t: f32| {
            let f = |x: i32, y: i32| (x as f32 + (y - x) as f32 * t.clamp(0.0, 1.0)) as i32;
            (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
        };
        let (spring_leaf, summer_leaf, autumn_leaf, winter_leaf) = ((120, 205, 90), (52, 140, 60), (214, 130, 32), (112, 80, 44));
        let (spring_grass, summer_grass, autumn_grass, winter_grass) = ((100, 190, 70), (88, 172, 64), (168, 150, 62), (130, 140, 110));
        let (leaf, grass, autumn) = match self.season() {
            Season::Spring => (mix(spring_leaf, summer_leaf, p), mix(spring_grass, summer_grass, p), 0.0),
            Season::Summer => (summer_leaf, summer_grass, 0.0),
            Season::Autumn => (mix(summer_leaf, autumn_leaf, p * 1.6), mix(summer_grass, autumn_grass, p), (p * 1.6).min(1.0)),
            Season::Winter => (mix(autumn_leaf, winter_leaf, p * 2.0), mix(autumn_grass, winter_grass, p), 1.0),
        };
        self.leaf_rgb = leaf;
        self.grass_rgb = grass;
        self.autumn = autumn;
    }

    // ----- time of day ------------------------------------------------------

    /// 0.0 is sunrise, 0.25 noon, 0.5 sunset, 0.75 midnight.
    pub fn phase(&self) -> f32 {
        (self.clock % DAY_STEPS) as f32 / DAY_STEPS as f32
    }

    pub fn set_phase(&mut self, p: f32) {
        let day = self.clock / DAY_STEPS;
        self.clock = day * DAY_STEPS + (p.rem_euclid(1.0) * DAY_STEPS as f32) as u32;
    }

    pub fn day_number(&self) -> u32 {
        self.clock / DAY_STEPS + 1
    }

    /// Strength of the sunlight, 0 at night.
    pub fn sun(&self) -> f32 {
        (self.phase() * std::f32::consts::TAU).sin().max(0.0)
    }

    /// Clock reading, e.g. "14:30" (sunrise is 06:00).
    pub fn clock_label(&self) -> String {
        let h = (self.phase() * 24.0 + 6.0) % 24.0;
        format!("{:02}:{:02}", h as u32, ((h.fract()) * 60.0) as u32)
    }

    /// Air temperature at ground level.
    pub fn surface_temp_c(&self) -> f32 {
        if let Some(t) = self.climate_fixed {
            return t;
        }
        let (peak, low) = (self.day_peak_c(), self.night_low_c());
        0.5 * (peak + low) + 0.5 * (peak - low) * (self.phase() * std::f32::consts::TAU).sin()
    }

    /// Temperature at the very top of the screen: it hovers around zero all the time.
    pub fn sky_top_c(&self) -> f32 {
        2.5 * (self.phase() * std::f32::consts::TAU).sin()
    }

    /// Air temperature at a row: it gets colder with altitude.
    pub fn ambient_c(&self, y: i32) -> f32 {
        if let Some(t) = self.climate_fixed {
            return t;
        }
        // The gradient starts at the top of the screen (around 0 °C) and runs down to
        // the air temperature at ground level.
        let f = (y as f32 / GROUND_Y).clamp(0.0, 1.0);
        let top = self.sky_top_c();
        top + (self.surface_temp_c() - top) * f
    }

    /// Where the sun (by day) sits in the world, in cells; matches the backdrop.
    pub fn sun_position(&self) -> (f32, f32) {
        let u = (self.phase() / 0.5).min(1.0);
        (W as f32 * (0.08 + 0.84 * u), H as f32 * (0.64 - 0.52 * (std::f32::consts::PI * u).sin()))
    }

    pub fn temp_c(&self, x: i32, y: i32) -> f32 {
        if Self::inb(x, y) { self.temp[Self::idx(x, y)] } else { self.ambient_c(y) }
    }

    /// Everything starts at the temperature of the air around it.
    pub fn reset_temps(&mut self) {
        self.revision += 1;
        for y in 0..H {
            let amb = self.ambient_c(y as i32);
            for x in 0..W {
                let i = y * W + x;
                self.temp[i] = match self.cells[i].m {
                    Mat::Fire => FIRE_C,
                    Mat::Ember => EMBER_C,
                    Mat::Lava => LAVA_C,
                    _ => amb,
                };
            }
        }
    }

    // ----- wind -------------------------------------------------------------

    /// Slowly changing wind, -1 (blowing left) to 1 (blowing right). It is never
    /// dead calm: the direction changes slowly but there is always a breeze.
    pub(super) fn update_wind(&mut self) {
        let t = self.ticks as f32;
        let tau = std::f32::consts::TAU;
        let base = 0.6 * (t * tau / 7200.0 + 1.3).sin() + 0.3 * (t * tau / 1900.0 + 0.4).sin() + 0.15 * (t / 300.0).sin();
        let base = base.clamp(-1.0, 1.0);
        self.wind = base.signum() * (0.3 + 0.7 * base.abs());
    }

    // ----- heat flow --------------------------------------------------------

    /// One thermal tick: conduction, air and sun forcing, fixed heat sources,
    /// and every material reaction that depends on temperature.
    pub(super) fn thermal_pass(&mut self) {
        self.thermal_flip = !self.thermal_flip;
        let horizontal = self.thermal_flip;

        // 1. Conduction between neighbours (alternating direction).
        let tb = tables();
        for y in 0..H {
            for x in 0..W {
                let i = y * W + x;
                let j = if horizontal {
                    if x + 1 >= W {
                        continue;
                    }
                    i + 1
                } else {
                    if y + 1 >= H {
                        continue;
                    }
                    i + W
                };
                let (ti, tj) = (self.temp[i], self.temp[j]);
                let d = tj - ti;
                if d.abs() < 0.02 {
                    continue;
                }
                let (mi, mj) = (self.cells[i].m as usize, self.cells[j].m as usize);
                // Flames and embers pour heat into whatever touches them.
                let k = if mi == Mat::Fire as usize || mi == Mat::Ember as usize || mj == Mat::Fire as usize || mj == Mat::Ember as usize {
                    0.25
                } else if mi == Mat::Lava as usize || mj == Mat::Lava as usize || mi == Mat::Molten as usize || mj == Mat::Molten as usize {
                    // Lava and molten metal soak into whatever they touch.
                    0.2
                } else {
                    tb.k[mi].min(tb.k[mj])
                };
                let flux = d * k * TS;
                self.temp[i] = ti + flux * tb.inv_c[mi];
                self.temp[j] = tj - flux * tb.inv_c[mj];
            }
        }

        // The sun radiates: the air and the ground are warmest near it and the
        // warmth fades with distance. Up in the sky it gets colder with altitude.
        let sun = self.sun();
        let (sx, sy) = self.sun_position();
        let gx: Vec<f32> = (0..W).map(|x| (-((x as f32 - sx) / (0.42 * W as f32)).powi(2)).exp()).collect();
        let gy: Vec<f32> = (0..H).map(|y| (-((y as f32 - sy) / (0.55 * H as f32)).powi(2)).exp()).collect();
        let fixed = self.climate_fixed.is_some();

        // 2. Air relaxes toward the ambient profile; fixed sources; reactions.
        let surf = self.surface_temp_c();
        let night_cold = surf < 5.0;
        for y in 0..H {
            let base = self.ambient_c(y as i32);
            let can_condense = (y as i32) < COLD_Y || base <= 5.0 || night_cold && (y as i32) < GROUND_Y as i32;
            for x in 0..W {
                let i = y * W + x;
                let m = self.cells[i].m;
                let t = self.temp[i];
                let amb = if fixed { base } else { base + 12.0 * sun * gx[x] * gy[y] };
                match m {
                    Mat::Empty => self.temp[i] = t + (amb - t) * 0.12 * TS,
                    Mat::Steam | Mat::Cloud | Mat::Snow | Mat::Hail | Mat::Smoke => self.temp[i] = t + (amb - t) * 0.03 * TS,
                    Mat::Fire => self.temp[i] = FIRE_C,
                    Mat::Ember => self.temp[i] = EMBER_C,
                    _ => {}
                }
                if tb.reactive[m as usize] {
                    self.react(i, x as i32, y as i32, m, self.temp[i], can_condense);
                }
            }
        }

        // 3. Sunlight comes down each column: clouds and steam soak up part of it
        //    (so they stay warm and do not condense), the rest reaches the ground.
        if sun > 0.0 {
            for x in 0..W {
                let mut light = sun * if fixed { 1.0 } else { 0.5 + 0.5 * gx[x] };
                let mut y = 0;
                while y < H && light > 0.02 {
                    let i = y * W + x;
                    let m = self.cells[i].m;
                    match m {
                        Mat::Empty => {}
                        Mat::Cloud | Mat::Steam => {
                            let target = surf + 12.0 * light * 0.5;
                            if target > self.temp[i] {
                                self.temp[i] += (target - self.temp[i]) * 0.04 * TS;
                            }
                            light *= 0.8;
                        }
                        Mat::Fire | Mat::Ember | Mat::Lava => {}
                        _ => {
                            let target = surf + 15.0 * light * absorb(m);
                            if target > self.temp[i] {
                                self.temp[i] += (target - self.temp[i]) * 0.05 * TS;
                            }
                            break;
                        }
                    }
                    y += 1;
                }
            }
        }
    }

    /// Material changes driven by temperature. Transformations consume their
    /// inputs: water that boils is gone, steam that condenses is a cloud.
    fn react(&mut self, i: usize, x: i32, y: i32, m: Mat, t: f32, can_condense: bool) {
        // Melting, boiling, freezing, setting: one rule for every material.
        let ph = phase(m);
        if let Some((th, to, rate)) = ph.up {
            if t >= th && self.chance(rate) {
                self.set(x, y, to);
                // The new phase takes the heat from whatever made it change: halfway
                // between the threshold and its hottest neighbour. That is how fresh
                // lava keeps melting the rock beside it, layer after layer, until the
                // heat has thinned out.
                let hottest = Self::neighbors(x, y)
                    .iter()
                    .filter(|&&(nx, ny)| Self::inb(nx, ny))
                    .map(|&(nx, ny)| self.temp[Self::idx(nx, ny)])
                    .fold(th, f32::max);
                self.temp[i] = (0.5 * (th + hottest)).min(1300.0);
                return;
            }
        }
        if let Some((th, to, rate)) = ph.down {
            if t <= th && self.chance(rate) {
                self.set(x, y, to);
                self.temp[i] = t.max(th);
                return;
            }
        }
        match m {
            Mat::Water | Mat::Molten => {}
            Mat::Ice | Mat::Snow | Mat::Hail => {
                if m == Mat::Snow && t < -2.0 {
                    // Deep, cold snow is pressed into ice (the way a glacier forms).
                    let above = (1..=3).filter(|d| matches!(self.raw(x, y - d), Mat::Snow | Mat::Ice | Mat::Hail)).count();
                    if (above >= 2 && self.chance(500)) || (t < -12.0 && above >= 1 && self.chance(4000)) {
                        self.set(x, y, Mat::Ice);
                    }
                }
            }
            Mat::Steam => {
                if t <= 45.0 && can_condense && self.chance(8) {
                    self.set(x, y, Mat::Cloud);
                }
            }
            Mat::Lava => {}
            Mat::Stone => {
                if t > -8.0 && t < 6.0 && self.chance(20_000) {
                    // Frost weathering: water in the cracks freezes and thaws, rock crumbles to sand.
                    let wet = Self::neighbors(x, y).iter().any(|&(nx, ny)| matches!(self.raw(nx, ny), Mat::Water | Mat::Ice));
                    if wet {
                        self.set(x, y, Mat::Sand);
                    }
                }
            }
            Mat::Sand | Mat::Shard => {}
            Mat::Glass => {
                // Thermal shock: neighbours at very different temperatures crack it.
                let mut worst = 0.0f32;
                for (nx, ny) in Self::neighbors(x, y) {
                    if Self::inb(nx, ny) {
                        worst = worst.max((self.temp[Self::idx(nx, ny)] - t).abs());
                    }
                }
                if worst > SHOCK_C && self.chance(6) {
                    self.shatter(x, y);
                }
            }
            Mat::Rust => {
                if t >= SMELT_C && self.chance(15) {
                    self.set(x, y, Mat::Iron);
                }
            }
            Mat::Iron => {
                if t >= STEEL_C && self.chance(10) {
                    if let Some((ax, ay)) = self.find_near(x, y, 3, Mat::Ash) {
                        self.set(ax, ay, Mat::Empty);
                        self.set(x, y, Mat::Steel);
                    }
                }
            }
            Mat::Wood | Mat::Trunk | Mat::Leaf | Mat::Branch | Mat::DeadLeaf => {
                if t >= WOOD_IGNITE_C && self.chance(10) {
                    self.set(x, y, Mat::Ember);
                }
            }
            Mat::Grass | Mat::Seed | Mat::Sapling => {
                if t >= WOOD_IGNITE_C - 40.0 && self.chance(10) {
                    self.set(x, y, Mat::Fire);
                }
            }
            Mat::Oil => {
                if t >= OIL_IGNITE_C && self.chance(3) {
                    self.ignite_oil(x, y);
                }
            }
            Mat::Gunpowder => {
                if t >= POWDER_IGNITE_C {
                    self.set(x, y, Mat::Fire);
                    self.explode(x, y, 4 * K + 4);
                }
            }
            _ => {}
        }
        let _ = i;
    }

    // ----- colours ----------------------------------------------------------

    /// False-colour temperature map: blue (cold) to red (hot) to white.
    pub fn thermal_color(&self, x: usize, y: usize) -> [u8; 4] {
        let i = y * W + x;
        if self.mask[i] != 0 {
            return [60, 60, 70, 255];
        }
        const STOPS: [(f32, [f32; 3]); 8] = [
            (-40.0, [20.0, 30.0, 120.0]),
            (0.0, [40.0, 110.0, 230.0]),
            (20.0, [60.0, 200.0, 180.0]),
            (60.0, [110.0, 220.0, 80.0]),
            (100.0, [240.0, 230.0, 60.0]),
            (300.0, [250.0, 140.0, 30.0]),
            (600.0, [240.0, 50.0, 30.0]),
            (1000.0, [255.0, 255.0, 255.0]),
        ];
        let t = self.temp[i].clamp(-40.0, 1000.0);
        let k = STOPS.iter().position(|s| s.0 >= t).unwrap_or(STOPS.len() - 1).max(1);
        let (t0, c0) = STOPS[k - 1];
        let (t1, c1) = STOPS[k];
        let f = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
        let mix = |a: f32, b: f32| (a + (b - a) * f) as u8;
        // Solid things are drawn a little darker than air so shapes stay readable.
        let dim = if self.cells[i].m == Mat::Empty { 0.78 } else { 1.0 };
        [(mix(c0[0], c1[0]) as f32 * dim) as u8, (mix(c0[1], c1[1]) as f32 * dim) as u8, (mix(c0[2], c1[2]) as f32 * dim) as u8, 255]
    }

    /// Density sample for the cloud/steam layer: premultiplied RGBA that the
    /// renderer blurs into soft, blobby puffs.
    pub fn gas_color(&self, x: usize, y: usize) -> [u8; 4] {
        let c = self.cells[y * W + x];
        let h = (x as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663);
        let v = ((h >> 9) & 31) as i32;
        let (r, g, b, a): (i32, i32, i32, i32) = match c.m {
            Mat::Cloud => (248, 250, 255, 200 + v),
            Mat::Steam => (226, 232, 242, 150 + v),
            Mat::Smoke => (74, 74, 82, 190 + v),
            _ => return [0, 0, 0, 0],
        };
        [(r * a / 255) as u8, (g * a / 255) as u8, (b * a / 255) as u8, a as u8]
    }
}
