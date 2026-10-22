//! Pressure that pushes things. The air's pressure comes from the wind solver (`flow.rs`); liquids
//! pass it on whole (Pascal: an incompressible liquid has the same excess pressure everywhere it
//! is connected to the open air) and the excess pressure then moves liquids, loose matter and
//! rigid blocks from high to low. The weight of the liquid itself is not counted here: gravity and
//! the buoyancy of the liquid already balance it.

use super::flow::{CELL, FH, FW};
use super::{density, Kind, World, H, W};

/// Excess pressure gradient (atmospheres per world cell) below which a liquid is not pushed.
const QUIET: f32 = 0.006;
/// The same for loose matter: a fire's hot air must not scatter the sand it is melting, a blast must.
const QUIET_LOOSE: f32 = 0.02;
/// How eagerly a gradient above it moves a cell of water (lighter matter moves more readily).
const PUSH: f32 = 40.0;

#[inline]
fn at(i: i32, j: i32) -> usize {
    j as usize * FW + i.rem_euclid(FW as i32) as usize
}

impl World {
    /// Re-computes the excess pressure (atmospheres over the ambient one) of every flow cell:
    /// air takes the wind solver's value, liquid cells share the pressure of the air above them.
    pub(super) fn update_pressure(&mut self) {
        let n = FW * FH;
        let (mut liquid, mut solid) = (vec![false; n], vec![false; n]);
        for j in 0..FH {
            for i in 0..FW {
                let (mut cnt, mut liq, mut sol) = (0, 0, 0);
                for dy in (0..CELL).step_by(2) {
                    let y = j * CELL + dy;
                    if y >= H {
                        break;
                    }
                    for dx in (0..CELL).step_by(2) {
                        cnt += 1;
                        match self.cells[y * W + i * CELL + dx].m.kind() {
                            Kind::Liquid => liq += 1,
                            Kind::Static | Kind::Powder => sol += 1,
                            _ => {}
                        }
                    }
                }
                liquid[j * FW + i] = liq * 2 >= cnt;
                solid[j * FW + i] = sol * 2 >= cnt && !liquid[j * FW + i];
            }
        }
        // Air: the solver's pressure. Liquid: first the air just above it, then shared out.
        for j in 0..FH {
            for i in 0..FW {
                let k = j * FW + i;
                self.dp[k] = if liquid[k] || solid[k] { 0.0 } else { self.flow.excess_at_cell(i, j) };
            }
        }
        for j in 0..FH as i32 {
            for i in 0..FW as i32 {
                let k = at(i, j);
                if !liquid[k] {
                    continue;
                }
                let mut up = j;
                while up > 0 && liquid[at(i, up)] {
                    up -= 1;
                }
                let u = at(i, up);
                self.dp[k] = if liquid[u] || solid[u] { 0.0 } else { self.dp[u] };
            }
        }
        for _ in 0..24 {
            for j in 0..FH as i32 {
                for i in 0..FW as i32 {
                    let k = at(i, j);
                    if !liquid[k] {
                        continue;
                    }
                    let (mut sum, mut cnt) = (0.0, 0.0);
                    for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let nj = j + dj;
                        if nj < 0 || nj >= FH as i32 {
                            continue;
                        }
                        let q = at(i + di, nj);
                        if !solid[q] {
                            sum += self.dp[q];
                            cnt += 1.0;
                        }
                    }
                    if cnt > 0.0 {
                        self.dp[k] = sum / cnt;
                    }
                }
            }
        }
        // Rock takes the pressure of what is around it, so the gradient has no hole in it.
        for j in 0..FH as i32 {
            for i in 0..FW as i32 {
                let k = at(i, j);
                if !solid[k] {
                    continue;
                }
                let (mut sum, mut cnt) = (0.0, 0.0);
                for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let nj = j + dj;
                    if nj >= 0 && nj < FH as i32 && !solid[at(i + di, nj)] {
                        sum += self.dp[at(i + di, nj)];
                        cnt += 1.0;
                    }
                }
                self.dp[k] = if cnt > 0.0 { sum / cnt } else { 0.0 };
            }
        }
        // What pushes is the part that changes quickly (a blast wave, a surge): the slowly varying
        // part is the steady overpressure of a plume or a fire, which the air flows round.
        for k in 0..n {
            self.dp_slow[k] += (self.dp[k] - self.dp_slow[k]) * 0.08;
            self.dpd[k] = self.dp[k] - self.dp_slow[k];
        }
    }

    /// Excess pressure at a world position, in atmospheres (bilinear between flow cells).
    pub fn excess_pressure_at(&self, x: f32, y: f32) -> f32 {
        Self::bilinear(&self.dp, x, y)
    }

    fn bilinear(field: &[f32], x: f32, y: f32) -> f32 {
        let fx = x / CELL as f32 - 0.5;
        let fy = (y / CELL as f32 - 0.5).clamp(0.0, FH as f32 - 1.001);
        let (i0, j0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - i0, fy - j0);
        let (i0, j0) = (i0 as i32, j0 as i32);
        let a = field[at(i0, j0)] * (1.0 - tx) + field[at(i0 + 1, j0)] * tx;
        let b = field[at(i0, j0 + 1)] * (1.0 - tx) + field[at(i0 + 1, j0 + 1)] * tx;
        a * (1.0 - ty) + b * ty
    }

    /// Gradient of the quickly changing excess pressure at a world position, in atmospheres per
    /// world cell: the part that pushes.
    pub fn pressure_gradient(&self, x: f32, y: f32) -> (f32, f32) {
        let h = CELL as f32;
        let gx = (Self::bilinear(&self.dpd, x + h, y) - Self::bilinear(&self.dpd, x - h, y)) / (2.0 * h);
        let gy = (Self::bilinear(&self.dpd, x, y + h) - Self::bilinear(&self.dpd, x, y - h)) / (2.0 * h);
        (gx, gy)
    }

    /// The pressure pushes: liquids and loose matter at the edge of a gradient slide towards low
    /// pressure, the lighter the quicker. Only flow cells with a real gradient are visited.
    pub(super) fn pressure_push(&mut self) {
        if !self.pressure_forces {
            return;
        }
        for j in 1..FH as i32 - 1 {
            for i in 0..FW as i32 {
                let gx = (self.dpd[at(i + 1, j)] - self.dpd[at(i - 1, j)]) / (2.0 * CELL as f32);
                let gy = (self.dpd[at(i, j + 1)] - self.dpd[at(i, j - 1)]) / (2.0 * CELL as f32);
                let g = (gx * gx + gy * gy).sqrt();
                if g < QUIET {
                    continue;
                }
                let (ux, uy) = (-gx / g, -gy / g);
                let (x0, y0) = (i * CELL as i32, j * CELL as i32);
                for dy in 0..CELL as i32 {
                    for dx in 0..CELL as i32 {
                        let (x, y) = (x0 + dx, y0 + dy);
                        let m = self.at(x, y);
                        if !matches!(m.kind(), Kind::Liquid | Kind::Powder) {
                            continue;
                        }
                        let quiet = if m.kind() == Kind::Liquid { QUIET } else { QUIET_LOOSE };
                        let p = ((g - quiet) * PUSH / density(m).max(0.2)).min(1.0);
                        if p <= 0.0 {
                            continue;
                        }
                        if (self.rand() % 1000) as f32 / 1000.0 >= p {
                            continue;
                        }
                        // Straight along the push, else along its horizontal part, else up and over
                        // (a blown grain hops onto its neighbour).
                        let (sx, sy) = (if ux.abs() > 0.2 { ux.signum() as i32 } else { 0 }, if uy.abs() > 0.2 { uy.signum() as i32 } else { 0 });
                        let tries: [(i32, i32); 3] = if sx == 0 { [(0, sy), (0, sy), (0, sy)] } else { [(sx, sy), (sx, 0), (sx, -1)] };
                        for (tx, ty) in tries {
                            if tx == 0 && ty == 0 {
                                continue;
                            }
                            let (nx, ny) = ((x + tx).rem_euclid(W as i32), y + ty);
                            if ny >= 0 && ny < H as i32 && matches!(self.at(nx, ny).kind(), Kind::Empty | Kind::Gas) {
                                self.swap((x, y), (nx, ny));
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Mat;

    fn floor(w: &mut World) {
        for x in 0..W as i32 {
            for y in 400..420 {
                w.set(x, y, Mat::Stone);
            }
        }
    }

    fn mean_x(w: &World, m: Mat) -> f32 {
        let (mut sum, mut n) = (0.0f32, 0.0f32);
        for y in 300..420 {
            for x in 0..W {
                if w.cells[y * W + x].m == m {
                    sum += x as f32;
                    n += 1.0;
                }
            }
        }
        sum / n.max(1.0)
    }

    #[test]
    fn a_pressure_wave_pushes_sand_away_from_the_blast() {
        let mut w = World::new_static();
        floor(&mut w);
        for x in 420..440 {
            for y in 396..400 {
                w.set(x, y, Mat::Sand);
            }
        }
        let before = mean_x(&w, Mat::Sand);
        w.flow.blast(395, 392, 40, 1.5);
        for _ in 0..60 {
            w.step();
        }
        let after = mean_x(&w, Mat::Sand);
        assert!(after > before + 1.5, "the sand is pushed away from the blast: {before} -> {after}");
    }

    #[test]
    fn no_pressure_difference_no_push() {
        let mut w = World::new_static();
        floor(&mut w);
        for x in 500..520 {
            for y in 396..400 {
                w.set(x, y, Mat::Sand);
            }
        }
        for x in 300..340 {
            for y in 390..400 {
                w.set(x, y, Mat::Water);
            }
        }
        let before: Vec<_> = w.cells.iter().map(|c| c.m as u8).collect();
        w.dpd.fill(0.0);
        for _ in 0..50 {
            w.pressure_push();
        }
        let after: Vec<_> = w.cells.iter().map(|c| c.m as u8).collect();
        assert!(before == after, "nothing moves without a gradient");
        // And the quiet threshold ignores the ordinary breeze.
        w.dp.fill(0.0);
        w.dpd[49 * FW + 63] = 0.01;
        for _ in 0..50 {
            w.pressure_push();
        }
        let after: Vec<_> = w.cells.iter().map(|c| c.m as u8).collect();
        assert!(before == after, "a faint gradient is below the threshold");
    }

    #[test]
    fn a_surge_of_air_pressure_drives_water_sideways() {
        let build = || {
            let mut w = World::new_static();
            floor(&mut w);
            for x in 300..420 {
                for y in 388..400 {
                    w.set(x, y, Mat::Water);
                }
            }
            w
        };
        let (mut blasted, mut control) = (build(), build());
        blasted.flow.blast(250, 380, 50, 1.5);
        for _ in 0..60 {
            blasted.step();
            control.step();
        }
        let (a, b) = (mean_x(&blasted, Mat::Water), mean_x(&control, Mat::Water));
        assert!(a > b + 0.5, "high pressure on the left pushes the water to the right: {a} vs {b}");
    }

    #[test]
    fn a_liquid_passes_the_air_pressure_down_to_its_depths() {
        let mut w = World::new_static();
        floor(&mut w);
        for x in 280..360 {
            for y in 376..400 {
                w.set(x, y, Mat::Water);
            }
        }
        w.flow.blast(320, 350, 40, 1.0);
        for _ in 0..3 {
            w.step();
        }
        w.update_pressure();
        let (top, deep) = (w.excess_pressure_at(320.0, 378.0), w.excess_pressure_at(320.0, 396.0));
        assert!(top > 0.1, "the surface feels the blast: {top}");
        assert!((deep - top).abs() < 0.35 * top, "and the water passes it on to the bottom: {top} vs {deep}");
    }
}

#[cfg(test)]
mod boom {
    use super::*;
    use crate::world::Mat;

    fn pool(forces: bool) -> World {
        let mut w = World::new_static();
        w.pressure_forces = forces;
        for x in 0..W as i32 {
            for y in 400..420 {
                w.set(x, y, Mat::Stone);
            }
        }
        for x in 300..660 {
            for y in 330..400 {
                w.set(x, y, Mat::Water);
            }
        }
        // Sand on the bottom and a little above the surface on the right.
        for x in 520..540 {
            for y in 396..400 {
                w.set(x, y, Mat::Sand);
            }
        }
        w
    }

    fn dump(w: &World, title: &str) {
        eprintln!("--- {title}");
        for y in (290..400).step_by(5) {
            let row: String = (360..600).step_by(3).map(|x| match w.cells[y * W + x].m {
                Mat::Water => '~',
                Mat::Sand => 's',
                Mat::Stone => '#',
                Mat::Steam | Mat::Cloud => 'o',
                Mat::Empty => '.',
                _ => '?',
            }).collect();
            eprintln!("{y:3} {row}");
        }
    }

    #[test]
    fn a_shallow_underwater_blast_throws_a_plume_a_deep_one_does_not() {
        let spray = |depth: i32| {
            let mut w = pool(true);
            for y in 290..400 {
                w.set(299, y, Mat::Stone);
                w.set(660, y, Mat::Stone);
            }
            w.explode(450, 330 + depth, 18);
            for _ in 0..3 {
                w.step();
            }
            (200..330).map(|y| (300..660).filter(|&x| w.cells[y * W + x].m == Mat::Water).count()).sum::<usize>()
        };
        let (shallow, deep) = (spray(25), spray(62));
        assert!(shallow > 60, "a plume of spray rises above the surface: {shallow}");
        assert!(deep < shallow / 4, "a deep blast only heaves the surface: {deep} vs {shallow}");
    }

    #[test]
    #[ignore]
    fn underwater_explosion() {
        for forces in [true, false] {
            let mut w = pool(forces);
            for y in 290..400 {
                w.set(299, y, Mat::Stone);
                w.set(660, y, Mat::Stone);
            }
            w.explode(450, 355, 18);
            eprintln!("===== pressure_forces = {forces}");
            for t in 1..=120 {
                w.step();
                if [3, 10, 20, 40, 120].contains(&t) {
                    let splash = (200..330).map(|y| (300..660).filter(|&x| w.cells[y * W + x].m == Mat::Water).count()).sum::<usize>();
                    let top = (200..400).find(|&y| (300..660).any(|x| w.cells[y * W + x].m == Mat::Water)).unwrap_or(0);
                    let total = (200..400).map(|y| (300..660).filter(|&x| w.cells[y * W + x].m == Mat::Water).count()).sum::<usize>();
                    let sand_x = { let v: Vec<usize> = (0..W * H).filter(|&i| w.cells[i].m == Mat::Sand).map(|i| i % W).collect(); v.iter().sum::<usize>() as f32 / v.len().max(1) as f32 };
                    let maxdp = (0..FW * FH).map(|k| w.dpd[k].abs()).fold(0.0f32, f32::max);
                    eprintln!("t={t:3} water {total} splash>y330 {splash} highest water y={top} sand mean x {sand_x:.1} quick-dp max {maxdp:.2}");
                }
            }
        }
    }
}
