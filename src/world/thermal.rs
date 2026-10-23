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
/// How much hotter burning charcoal glows than burning wood.
pub const COAL_EXTRA_C: f32 = 150.0;
pub const LAVA_C: f32 = 1100.0;
pub const BOIL_C: f32 = 100.0;
/// Water freezes and ice melts at 0 °C. What keeps it from flickering between the two is the
/// latent heat: a cell has to give up (or take in) `LATENT_FUSION` before it changes.
pub const FREEZE_C: f32 = 0.0;
pub const MELT_C: f32 = 0.0;
/// Heat (°C times heat capacity, the units of the heat flow) a cell of ice takes in to melt,
/// and a cell of water gives up to freeze. Real water: 80 °C worth of heating.
pub const LATENT_FUSION: f32 = 240.0;
/// Heat a cell of boiling water takes in to become steam. Real water: about 540 °C worth;
/// the game uses a third of it so that a pot still boils dry in a reasonable time.
pub const LATENT_VAPOUR: f32 = 540.0;
/// Heat capacities of water and ice (as in `props`).
const C_WATER: f32 = 3.0;
const C_ICE: f32 = 2.0;
/// Lava turns to rock once it has cooled below this.
pub const LAVA_SOLID_C: f32 = 390.0;
pub const ROCK_MELT_C: f32 = 540.0;
pub const GLASS_C: f32 = 600.0;
/// Metal melts in fresh lava or at the heart of a closed charcoal forge, and sets again once it
/// cools below `METAL_SOLID_C`. (Real iron melts at 1538 °C; the game squeezes the hot end of the
/// scale the same way for rock and sand, so that a forge can get there.)
pub const METAL_MELT_C: f32 = 900.0;
pub const METAL_SOLID_C: f32 = 820.0;
pub const STEEL_C: f32 = 400.0;
/// Clay turns to terracotta once it has been this hot.
pub const KILN_C: f32 = 550.0;
pub const SMELT_C: f32 = 400.0;
pub const WOOD_IGNITE_C: f32 = 300.0;
/// Charcoal catches fire in the air from this temperature.
pub const CHARCOAL_IGNITE_C: f32 = 300.0;
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
        Mat::Clay => (0.1, 2.4),
        Mat::Terracotta => (0.14, 3.0),
        Mat::Iron | Mat::Steel => (0.5, 2.5),
        Mat::Glass | Mat::Shard => (0.08, 3.0),
        Mat::Lava => (0.2, 400.0),
        Mat::Molten | Mat::MoltenSteel => (0.5, 300.0),
        Mat::Fire => (0.4, 1.0),
        Mat::Ember => (0.25, 1.0),
        Mat::Gunpowder | Mat::Saltpeter | Mat::Ash | Mat::Rust => (0.04, 1.2),
        Mat::Charcoal => (0.04, 1.0),
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
                Mat::Water | Mat::Ice | Mat::Snow | Mat::Hail | Mat::Steam | Mat::Lava | Mat::Molten | Mat::MoltenSteel | Mat::Stone | Mat::Sand
                    | Mat::Shard | Mat::Glass | Mat::Rust | Mat::Iron | Mat::Steel | Mat::Wood | Mat::Trunk | Mat::Leaf | Mat::Branch | Mat::DeadLeaf
                    | Mat::Grass | Mat::Seed | Mat::Sapling | Mat::Gunpowder | Mat::Oil | Mat::Clay | Mat::Charcoal
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
        // ice <-> water <-> steam: see `latent_heat`, which also counts the heat they take in.
        // rock <-> lava
        Mat::Stone => Phase { up: Some((ROCK_MELT_C, Mat::Lava, 30)), down: None },
        Mat::Lava => Phase { up: None, down: Some((LAVA_SOLID_C, Mat::Stone, 20)) },
        // iron <-> molten iron, steel <-> molten steel (fresh lava or a closed charcoal forge)
        Mat::Iron => Phase { up: Some((METAL_MELT_C, Mat::Molten, 40)), down: None },
        Mat::Molten => Phase { up: None, down: Some((METAL_SOLID_C, Mat::Iron, 20)) },
        Mat::Steel => Phase { up: Some((METAL_MELT_C, Mat::MoltenSteel, 40)), down: None },
        Mat::MoltenSteel => Phase { up: None, down: Some((METAL_SOLID_C, Mat::Steel, 20)) },
        // sand fuses into glass
        Mat::Sand | Mat::Shard => Phase { up: Some((GLASS_C, Mat::Glass, 15)), down: None },
        _ => none,
    }
}

/// The temperature a freshly painted state starts at, when it is not the air's: a state only
/// lasts at a temperature where it belongs (molten metal poured cold would set at once).
pub fn paint_temp(m: Mat) -> Option<f32> {
    match m {
        Mat::Lava => Some(LAVA_C),
        Mat::Molten | Mat::MoltenSteel => Some(METAL_MELT_C + 200.0),
        Mat::Ice | Mat::Snow => Some(-15.0),
        Mat::Steam => Some(95.0),
        _ => None,
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

    /// Slowly changing wind, -1 (blowing left) to 1 (blowing right). With the default settings it is
    /// never dead calm: the direction changes slowly but there is always a breeze. The player can
    /// cap its speed, make the direction change more or less often, and make the weather calmer.
    pub(super) fn update_wind(&mut self) {
        self.wind_clock += self.wind_changes;
        let t = self.wind_clock;
        let tau = std::f32::consts::TAU;
        let base = 0.6 * (t * tau / 7200.0 + 1.3).sin() + 0.3 * (t * tau / 1900.0 + 0.4).sin() + 0.15 * (t / 300.0).sin();
        let base = base.clamp(-1.0, 1.0);
        let windy = 0.3 + 0.7 * base.abs();
        let calm = 0.04 + 0.12 * base.abs();
        let mag = calm + (windy - calm) * self.windiness.clamp(0.0, 1.0);
        self.wind = base.signum() * mag * self.wind_max.clamp(0.0, 1.0);
    }

    // ----- heat flow --------------------------------------------------------

    /// How many times longer a thermal pass covers when the simulation is simplified.
    pub(super) fn thermal_scale(&self) -> f32 {
        match self.lod {
            0 | 1 => 1.0,
            _ => 2.0,
        }
    }

    /// `chance(n)` for a thermal rate: with a simplified (less frequent) pass the odds are scaled up.
    fn chance_t(&mut self, n: u32) -> bool {
        let n = (n as f32 / self.thermal_scale()).round().max(1.0) as u32;
        self.chance(n)
    }

    /// One thermal tick: conduction, air and sun forcing, fixed heat sources,
    /// and every material reaction that depends on temperature.
    pub(super) fn thermal_pass(&mut self) {
        // When the game has to save time the pass runs half as often (see `lod`): each run then
        // covers twice the time, so every rate is doubled to keep the same speed of change.
        let scale = self.thermal_scale();
        let ts = TS * scale;
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
                // Simplified: air next to air only exchanges heat where there is a real difference
                // (a hot plume); the rest of the sky just relaxes towards its surroundings below.
                if self.lod >= 3 && mi == Mat::Empty as usize && mj == Mat::Empty as usize && d.abs() < 4.0 {
                    continue;
                }
                // Flames and embers pour heat into whatever touches them.
                let k = if mi == Mat::Fire as usize || mi == Mat::Ember as usize || mj == Mat::Fire as usize || mj == Mat::Ember as usize {
                    0.25
                } else if mi == Mat::Lava as usize || mj == Mat::Lava as usize || mi == Mat::Molten as usize || mj == Mat::Molten as usize || mi == Mat::MoltenSteel as usize || mj == Mat::MoltenSteel as usize {
                    // Lava and molten metal soak into whatever they touch.
                    0.2
                } else {
                    tb.k[mi].min(tb.k[mj])
                };
                // (Capped so that a longer covered time can never overshoot and oscillate.)
                let flux = d * (k * ts).min(0.9);
                self.temp[i] = ti + flux * tb.inv_c[mi];
                self.temp[j] = tj - flux * tb.inv_c[mj];
            }
        }

        // Every other pass, with twice the reach: the wind is slow next to the thermal clock.
        if horizontal {
            self.carry_air_heat();
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
                    Mat::Empty => self.temp[i] = t + (amb - t) * 0.12 * ts,
                    Mat::Steam | Mat::Cloud | Mat::Snow | Mat::Hail | Mat::Smoke => self.temp[i] = t + (amb - t) * 0.03 * ts,
                    Mat::Fire => self.temp[i] = FIRE_C,
                    // Embers a draught cannot reach glow hotter (and last longer, see `step_ember`).
                    Mat::Ember => {
                        let open = Self::neighbors(x as i32, y as i32).iter().filter(|&&(nx, ny)| matches!(self.at(nx, ny).kind(), super::Kind::Empty | super::Kind::Gas)).count();
                        // Charcoal burns hotter than wood: nothing is left in it but carbon.
                        let glow = EMBER_C + 300.0 * (1.0 - open as f32 / 4.0);
                        let coal = self.cells[i].life >= super::COAL_LIFE;
                        self.temp[i] = if coal { glow + COAL_EXTRA_C } else { glow };
                    }
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
                                self.temp[i] += (target - self.temp[i]) * 0.04 * ts;
                            }
                            light *= 0.8;
                        }
                        Mat::Fire | Mat::Ember | Mat::Lava => {}
                        _ => {
                            let target = surf + 15.0 * light * absorb(m);
                            if target > self.temp[i] {
                                self.temp[i] += (target - self.temp[i]) * 0.05 * ts;
                                // Sunlight on ice or water goes into melting or boiling it first.
                                if matches!(m, Mat::Water | Mat::Ice | Mat::Snow | Mat::Hail) {
                                    self.latent_heat(i, x as i32, y as i32, m, self.temp[i]);
                                }
                            }
                            break;
                        }
                    }
                    y += 1;
                }
            }
        }
    }

    #[cfg(test)]
    pub(super) fn carry_air_heat_for_bench(&mut self) {
        self.carry_air_heat();
    }

    /// The wind carries warmth: every free air cell takes (bilinearly) the temperature of the air
    /// upwind of it, so a hot plume is blown sideways and a cold front sweeps across the map. Gas
    /// cells (steam, smoke, clouds) already carry their own temperature when the flow moves them.
    /// Wind also strips heat from whatever it blows over (convection): the faster, the quicker.
    /// With `lod >= 1` it works on blocks of 2 x 2 cells.
    fn carry_air_heat(&mut self) {
        let old = self.temp.clone();
        let reach = 2.0 * THERMAL_EVERY as f32 * self.thermal_scale();
        let stride = if self.lod >= 1 { 2 } else { 1 };
        let tb = tables();
        let cell = super::flow::CELL;
        let (fw, fh) = (super::flow::FW, super::flow::FH);
        for bj in 0..fh {
            for bi in 0..fw {
                // One wind per block of the flow grid: it is as fine as the wind itself, and the
                // upwind offset is the same for the whole block.
                let (vx, vy) = self.flow.at_cell(bi, bj);
                let speed = vx.abs() + vy.abs();
                if speed < 0.05 {
                    continue;
                }
                let (ox, oy) = (-vx * reach, -vy * reach);
                let (ix, iy) = (ox.floor(), oy.floor());
                let (tx, ty) = (ox - ix, oy - iy);
                let (ix, iy) = (ix as i32, iy as i32);
                let gust = speed.min(1.5) * 0.02;
                for y in (bj * cell..((bj + 1) * cell).min(H)).step_by(stride) {
                    let sy0 = (y as i32 + iy).clamp(0, H as i32 - 2) as usize;
                    for x in (bi * cell..(bi + 1) * cell).step_by(stride) {
                        let i = y * W + x;
                        if self.cells[i].m != Mat::Empty {
                            continue;
                        }
                        let sx0 = (x as i32 + ix).rem_euclid(W as i32 - 1) as usize;
                        // Free air is read as it is; anything else counts as the cell's own temperature.
                        let air = |j: usize| if self.cells[j].m == Mat::Empty { old[j] } else { old[i] };
                        let top = air(sy0 * W + sx0) * (1.0 - tx) + air(sy0 * W + sx0 + 1) * tx;
                        let bot = air((sy0 + 1) * W + sx0) * (1.0 - tx) + air((sy0 + 1) * W + sx0 + 1) * tx;
                        let upwind = top * (1.0 - ty) + bot * ty;
                        self.temp[i] = old[i] + (upwind - old[i]) * 0.7;
                        if stride == 2 {
                            // Simplified: the other three cells of the 2 x 2 block follow.
                            let t = self.temp[i];
                            for j in [i + 1, i + W, i + W + 1] {
                                if j < W * H && self.cells[j].m == Mat::Empty {
                                    self.temp[j] = t;
                                }
                            }
                        }
                        // Convection: the moving air also cools or warms the solid or liquid beside it.
                        for j in [i + 1, i.wrapping_sub(1), i + W, i.wrapping_sub(W)] {
                            if j >= W * H {
                                continue;
                            }
                            let m = self.cells[j].m;
                            if m == Mat::Empty || matches!(m.kind(), super::Kind::Gas | super::Kind::Flame) || matches!(m, Mat::Fire | Mat::Ember | Mat::Lava) {
                                continue;
                            }
                            let flux = (self.temp[i] - self.temp[j]) * gust;
                            self.temp[j] += flux * tb.inv_c[m as usize];
                            self.temp[i] -= flux * tb.inv_c[Mat::Empty as usize].min(1.0);
                        }
                    }
                }
            }
        }
    }

    /// Water, ice and snow change state only once they have taken in (or given up) their latent
    /// heat. Meanwhile their temperature stays put at the change point: boiling water stays at
    /// 100 °C (more under pressure) however hot the fire, melting ice stays at 0 °C. Heat that
    /// went in and was not enough is given back if the cell cools again. Returns true when the
    /// cell changed state.
    fn latent_heat(&mut self, i: usize, x: i32, y: i32, m: Mat, t: f32) -> bool {
        if m == Mat::Water {
            // Water boils later under pressure (a pressure cooker, the deep of a lake) and earlier
            // in a partial vacuum.
            let boil = super::boil_c(self.liq_p[i]);
            let stored = self.latent[i];
            if t > boil {
                self.latent[i] += (t - boil) * C_WATER;
                self.temp[i] = boil;
                if self.latent[i] >= LATENT_VAPOUR {
                    self.set(x, y, Mat::Steam);
                    self.temp[i] = boil + 1.0;
                    return true;
                }
            } else if t < FREEZE_C {
                self.latent[i] += (t - FREEZE_C) * C_WATER;
                self.temp[i] = FREEZE_C;
                if self.latent[i] <= -LATENT_FUSION {
                    self.set(x, y, Mat::Ice);
                    self.temp[i] = FREEZE_C;
                    return true;
                }
            } else if stored > 0.0 {
                // Half-boiled water that cools gives its heat back first.
                let dt = (stored / C_WATER).min(boil - t);
                self.temp[i] = t + dt;
                self.latent[i] -= dt * C_WATER;
            } else if stored < 0.0 {
                // Half-frozen water that warms takes back the cold first.
                let dt = (-stored / C_WATER).min(t - FREEZE_C);
                self.temp[i] = t - dt;
                self.latent[i] += dt * C_WATER;
            }
            return false;
        }
        // Ice, snow, hail.
        if t > MELT_C {
            self.latent[i] += (t - MELT_C) * C_ICE;
            self.temp[i] = MELT_C;
            if self.latent[i] >= LATENT_FUSION {
                self.set(x, y, Mat::Water);
                self.temp[i] = MELT_C;
                return true;
            }
        } else if self.latent[i] > 0.0 && t < MELT_C {
            let dt = (self.latent[i] / C_ICE).min(MELT_C - t);
            self.temp[i] = t + dt;
            self.latent[i] -= dt * C_ICE;
        }
        false
    }

    /// Material changes driven by temperature. Transformations consume their
    /// inputs: water that boils is gone, steam that condenses is a cloud.
    fn react(&mut self, i: usize, x: i32, y: i32, m: Mat, t: f32, can_condense: bool) {
        if matches!(m, Mat::Water | Mat::Ice | Mat::Snow | Mat::Hail) && self.latent_heat(i, x, y, m, t) {
            return;
        }
        // Melting, boiling, freezing, setting: one rule for every material.
        let ph = phase(m);
        if let Some((th, to, rate)) = ph.up {
            if t >= th && self.chance_t(rate) {
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
            if t <= th && self.chance_t(rate) {
                self.set(x, y, to);
                self.temp[i] = t.max(th);
                return;
            }
        }
        match m {
            Mat::Water | Mat::Molten | Mat::MoltenSteel => {}
            Mat::Ice | Mat::Snow | Mat::Hail => {
                if m == Mat::Snow && t < -2.0 {
                    // Deep, cold snow is pressed into ice (the way a glacier forms).
                    let above = (1..=3).filter(|d| matches!(self.raw(x, y - d), Mat::Snow | Mat::Ice | Mat::Hail)).count();
                    if (above >= 2 && self.chance_t(500)) || (t < -12.0 && above >= 1 && self.chance_t(4000)) {
                        self.set(x, y, Mat::Ice);
                    }
                }
            }
            Mat::Steam => {
                if t <= 45.0 && can_condense && self.chance_t(8) {
                    self.set(x, y, Mat::Cloud);
                }
            }
            Mat::Lava => {}
            Mat::Stone => {
                if t > -8.0 && t < 6.0 && self.chance_t(20_000) {
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
                if worst > SHOCK_C && self.chance_t(6) {
                    self.shatter(x, y);
                }
            }
            Mat::Clay => {
                if t >= KILN_C && self.chance_t(12) {
                    self.set(x, y, Mat::Terracotta);
                }
            }
            Mat::Rust => {
                // Rust is iron oxide: carbon takes its oxygen away. The carbon is charcoal close
                // by, or the carbon monoxide of a wood fire's flames and embers right against it.
                if t >= SMELT_C && self.chance_t(15) {
                    if let Some((cx, cy)) = self.find_near(x, y, 2, Mat::Charcoal) {
                        if self.chance(3) {
                            self.set(cx, cy, Mat::Empty);
                        }
                        self.set(x, y, Mat::Iron);
                    } else if Self::neighbors(x, y).iter().any(|&(nx, ny)| matches!(self.raw(nx, ny), Mat::Fire | Mat::Ember)) {
                        self.set(x, y, Mat::Iron);
                    }
                }
            }
            Mat::Iron => {
                // Hot iron soaks up carbon from the charcoal beside it: steel.
                // Glowing coals close by give it carbon too (the smith hardens iron in the forge's
                // coals), and burn away as they do.
                if t >= STEEL_C && self.chance_t(10) {
                    if let Some((cx, cy)) = self.find_near(x, y, 3, Mat::Charcoal).or_else(|| self.find_near(x, y, 3, Mat::Ember)) {
                        self.set(cx, cy, Mat::Empty);
                        self.set(x, y, Mat::Steel);
                    }
                }
            }
            Mat::Wood | Mat::Trunk | Mat::Leaf | Mat::Branch | Mat::DeadLeaf => {
                // With air it burns; shut away from the air it only chars (pyrolysis): the
                // water and the tars leave and almost pure carbon stays, charcoal.
                if t >= WOOD_IGNITE_C {
                    let air = self.breathes(x, y);
                    if air && self.chance_t(10) {
                        self.set(x, y, Mat::Ember);
                    } else if !air && self.chance_t(40) {
                        self.set(x, y, Mat::Charcoal);
                    }
                }
            }
            Mat::Charcoal => {
                if t >= CHARCOAL_IGNITE_C && self.chance_t(20) {
                    self.light_charcoal(x, y, 1);
                }
            }
            Mat::Grass | Mat::Seed | Mat::Sapling => {
                if t >= WOOD_IGNITE_C - 40.0 && self.chance_t(10) {
                    self.set(x, y, Mat::Fire);
                }
            }
            Mat::Oil => {
                if t >= OIL_IGNITE_C && self.chance_t(3) {
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

#[cfg(test)]
mod carry_tests {
    use super::*;

    #[test]
    fn the_wind_blows_warmth_downwind() {
        let mut w = World::new_static();
        w.climate_fixed = Some(10.0);
        w.reset_temps();
        w.flow.pin_uniform(0.25, 0.0);
        let (x0, y) = (300usize, 100usize);
        for dx in 0..12 {
            for dy in 0..12 {
                w.temp[(y + dy) * W + x0 + dx] = 80.0;
            }
        }
        let mean = |w: &World, x: usize| (0..12).map(|dy| w.temp[(y + dy) * W + x]).sum::<f32>() / 12.0;
        let mut passed = 0.0f32;
        for _ in 0..40 {
            w.carry_air_heat();
            passed = passed.max(mean(&w, x0 + 30));
        }
        assert!(passed > 20.0, "the warm patch travelled downwind past x0+30: {passed}");
        assert!(mean(&w, x0 - 30) < 11.0, "nothing upwind: {}", mean(&w, x0 - 30));
    }
}

#[cfg(test)]
mod bench_tests {
    use super::*;

    #[test]
    #[ignore]
    fn bench_air_heat() {
        let mut w = World::new_static();
        w.flow.pin_uniform(0.5, 0.2);
        let t = std::time::Instant::now();
        for _ in 0..30 {
            w.carry_air_heat();
        }
        eprintln!("carry_air_heat: {:?} per call", t.elapsed() / 30);
        let t = std::time::Instant::now();
        for _ in 0..30 {
            w.thermal_pass();
        }
        eprintln!("thermal_pass total: {:?} per call", t.elapsed() / 30);
        let t = std::time::Instant::now();
        for i in 0..100 {
            w.flow.step(0.5, i);
        }
        eprintln!("flow.step: {:?} per call", t.elapsed() / 100);
    }
}

#[cfg(test)]
mod latent_tests {
    use super::*;

    #[test]
    fn boiling_water_stays_at_100_until_it_has_all_turned_to_steam() {
        let mut w = World::new_static();
        // A pot: a terracotta bottom over a fire that never goes out, water in it, open to the air.
        // (Not iron: iron rusts in water and the pot would leak.)
        for x in 200..230 {
            w.set(x, 300, Mat::Terracotta);
            for y in 285..300 {
                w.set(x, y, Mat::Water);
            }
        }
        for y in 285..301 {
            w.set(199, y, Mat::Terracotta);
            w.set(230, y, Mat::Terracotta);
        }
        let (mut hottest, mut steam_at) = (0.0f32, None);
        for t in 0..6000 {
            for x in 200..230 {
                w.set(x, 301, Mat::Fire);
            }
            w.step();
            for i in 0..W * H {
                if w.cells[i].m == Mat::Water {
                    // (Deeper down the pressure makes the boiling point a little higher.)
                    hottest = hottest.max(w.temp[i] - super::super::boil_c(w.liq_p[i]) + 100.0);
                }
            }
            if steam_at.is_none() && w.cells.iter().any(|c| c.m == Mat::Steam) {
                steam_at = Some(t);
            }
        }
        assert!(hottest < 102.0, "boiling water never gets hotter than its boiling point: {hottest}");
        let t = steam_at.expect("it boils in the end");
        assert!(t > 30, "but only once it has taken in the latent heat: steam after {t} steps");
    }

    #[test]
    fn melting_ice_stays_at_0_and_water_freezes_at_0() {
        let mut w = World::new_static();
        w.climate_fixed = Some(20.0);
        w.reset_temps();
        for x in 200..220 {
            w.set(x, 300, Mat::Stone);
            for y in 290..300 {
                w.set(x, y, Mat::Ice);
                w.temp[y as usize * W + x as usize] = -5.0;
            }
        }
        let mut seen_zero = false;
        for _ in 0..3000 {
            w.step();
            let i = 295 * W + 210;
            if w.cells[i].m == Mat::Ice && w.temp[i].abs() < 0.01 {
                seen_zero = true;
            }
            assert!(w.cells[i].m != Mat::Ice || w.temp[i] <= 0.0 + 1e-3, "ice is never above 0 °C: {}", w.temp[i]);
        }
        assert!(seen_zero, "ice warming up stops at 0 °C while it melts");
        // Water a little below freezing turns to ice (it used to need -6 °C).
        let mut c = World::new_static();
        c.climate_fixed = Some(-3.0);
        c.reset_temps();
        // A terracotta pot: stone would crumble in the frost and let the water out.
        for y in 294..301 {
            c.set(199, y, Mat::Terracotta);
            c.set(220, y, Mat::Terracotta);
        }
        for x in 200..220 {
            c.set(x, 300, Mat::Terracotta);
            for y in 296..300 {
                c.set(x, y, Mat::Water);
                c.temp[y as usize * W + x as usize] = 2.0;
            }
        }
        let mut froze = false;
        for t in 0..8000 {
            c.step();
            if c.cells.iter().any(|m| m.m == Mat::Ice) {
                froze = true;
                break;
            }
        }
        assert!(froze, "water at -3 °C freezes");
    }
}
