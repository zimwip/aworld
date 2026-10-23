//! The edges of the world: every system run against the sides, the bottom row and the corners.
//! `at` reads Empty past the sides and the bottom, so "is the next cell free?" is true out
//! there too, and a cell index taken after it runs past the grid. These scenes put every kind of
//! matter, heat, charge, blasts, bolts and rigid blocks along the borders and step the world;
//! debug builds check every index (`World::idx`), so a slip shows up here and not in a game.

use super::*;

/// Cells along the four borders, `d` cells deep, corners included.
fn border(d: i32) -> Vec<(i32, i32)> {
    let (w, h) = (W as i32, H as i32);
    let mut v = vec![];
    for y in 0..h {
        for x in 0..w {
            if x < d || y < d || x >= w - d || y >= h - d {
                v.push((x, y));
            }
        }
    }
    v
}

fn corners() -> [(i32, i32); 4] {
    let (w, h) = (W as i32 - 1, H as i32 - 1);
    [(0, 0), (w, 0), (0, h), (w, h)]
}

/// Edge and corner spots for blasts and bolts: the corners, the middles of each side, and just
/// outside the grid (a pointer dragged past the window).
fn spots() -> Vec<(i32, i32)> {
    let (w, h) = (W as i32, H as i32);
    let mut v: Vec<(i32, i32)> = corners().to_vec();
    v.extend([(w / 2, 0), (w / 2, h - 1), (0, h / 2), (w - 1, h / 2)]);
    v.extend([(-3, h + 2), (w + 2, h + 2), (-5, -5), (w + 4, h / 2), (w / 3, h + 6)]);
    v
}

/// A world with `m` in a band along every border, hot or cold, charged, with clouds above.
fn scene(m: Mat, hot: bool) -> World {
    let mut w = World::new();
    for (x, y) in border(3) {
        w.set(x, y, m);
        let i = World::idx(x, y);
        if hot {
            w.temp[i] = 900.0;
        }
        // Packed vapour and soaked matter right at the edge.
        if matches!(m, Mat::Steam | Mat::Cloud) {
            w.moisture[i] = VAPOUR_UNITS * 6;
        }
        if matches!(m, Mat::Dirt | Mat::Grass | Mat::Mud) {
            w.moisture[i] = SOIL_FULL * 2;
        }
        // Saplings and seeds with a budget left, to grow into the sides and the top.
        if matches!(m, Mat::Sapling | Mat::Seed) {
            w.growth[i] = 60;
        }
    }
    // Clouds under the top edge to draw charge and throw bolts at the corners.
    for x in 0..W as i32 {
        for y in 4..8 {
            if w.raw(x, y) == Mat::Empty {
                w.set(x, y, Mat::Cloud);
            }
        }
    }
    for q in w.charge.iter_mut() {
        *q = 0.8;
    }
    for q in w.mcharge.iter_mut() {
        *q = -1.2;
    }
    w
}

/// Rigid blocks over the corners and along the bottom row, shifted one cell each call.
fn blocks(w: &mut World, t: i32) {
    w.begin_blocks(vec![Mat::Wood, Mat::Iron, Mat::Stone], vec![false, true, false]);
    let (wd, h) = (W as i32, H as i32);
    for (k, (cx, cy)) in corners().iter().enumerate() {
        for dy in -4..4 {
            for dx in -4..4 {
                w.mark_block(cx + dx + t % 3 - 1, cy + dy, k % 3);
            }
        }
    }
    for x in (0..wd).step_by(7) {
        w.mark_block(x + t % 2, h - 1, 2);
    }
    w.end_blocks();
}

fn run(m: Mat, hot: bool, steps: i32) {
    let mut w = scene(m, hot);
    let spots = spots();
    for t in 0..steps {
        if t % 4 == 0 {
            blocks(&mut w, t);
        }
        if t % 25 == 5 {
            let (x, y) = spots[(t / 25) as usize % spots.len()];
            w.explode(x, y, 18);
        }
        if t % 25 == 12 {
            let (x, y) = spots[(t / 25 + 3) as usize % spots.len()];
            w.detonate(x, y, blast::Charge::powder(40.0));
        }
        if t % 30 == 17 {
            let (x, y) = spots[(t / 30) as usize % spots.len()];
            w.strike(x, y);
        }
        if t % 40 == 20 {
            // The brush past the edges too.
            let (x, y) = spots[(t / 40 + 1) as usize % spots.len()];
            w.paint(x, y, 6, m);
            w.paint(x, y, 4, Mat::Dynamite);
        }
        w.step();
    }
}

#[test]
fn every_matter_hot_and_cold_along_the_edges_and_in_the_corners() {
    for &m in Mat::ALL.iter() {
        for hot in [false, true] {
            run(m, hot, 120);
        }
    }
}

#[test]
fn a_full_boiler_on_the_bottom_row_spreads_its_vapour_without_leaving_the_grid() {
    // The crash this file was written for: packed steam on the last row picked the cell below
    // it (outside the grid, but Empty by `at`) and read its temperature.
    let mut w = World::new();
    for (x, y) in border(1) {
        w.set(x, y, Mat::Steam);
        let i = World::idx(x, y);
        w.moisture[i] = VAPOUR_UNITS * 8;
        w.temp[i] = 400.0;
    }
    for _ in 0..200 {
        w.step();
    }
}

#[test]
fn a_mixed_world_full_to_the_edges_runs_long() {
    // Every matter at once, in stripes that touch all four borders, hot in places.
    let mut w = World::new();
    let all = Mat::ALL;
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            let m = all[((x / 5 + y / 7) as usize) % all.len()];
            w.set(x, y, m);
            if (x + y) % 11 == 0 {
                w.temp[World::idx(x, y)] = 1500.0;
            }
        }
    }
    let spots = spots();
    for t in 0..400 {
        if t % 4 == 0 {
            blocks(&mut w, t);
        }
        if t % 20 == 3 {
            let (x, y) = spots[(t / 20) as usize % spots.len()];
            w.explode(x, y, 30);
            w.strike(x, y);
        }
        w.step();
    }
}
