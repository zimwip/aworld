//! The colours of the world: of each cell as the game draws it, and of the views (pressure,
//! humidity) and their scales.

use super::*;

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

pub(super) fn ramp(stops: &[(f32, [f32; 3])], v: f32) -> (u8, u8, u8) {
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
pub(super) fn glow((r, g, b): (i32, i32, i32), t: f32) -> (i32, i32, i32, i32) {
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

impl World {
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
                // A cell of the field that is not full is water mixed with air: the less water,
                // the clearer it shows, and the paler (spray and the froth of a splash are
                // whitish, not a deeper blue).
                let q = self.water_fill(y * W() + x).min(field::FULL) as i32 * 256 / field::FULL as i32;
                let pale = |c: i32, to: i32| c + (to - c) * (256 - q) / 256;
                (pale(r, 200), pale(g, 222), pale(b, 240), 205 * q / 256)
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
                    // Humus is what makes earth dark: as it burns away, the earth pales towards
                    // the colour of the sand it is made of.
                    let o = 256 - 256 * self.organic[y * W() + x].min(SOIL_ORGANIC) as i32 / SOIL_ORGANIC as i32;
                    let mix = |a: i32, s: i32| a + (s - a) * o / 256;
                    (mix(112 + v, 214 + v) * k / 256, mix(78 + v / 2, 186 + v) * k / 256, mix(52 + v / 2, 118 + v) * k / 256, 255)
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
            // Glowing orange, half see-through.
            Mat::MoltenGlass => (255, 138 + flick * 2 + v, 52 + flick, 225),
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
