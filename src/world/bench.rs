//! The performance bench: a few busy scenes, each run for a while, with the time of every phase
//! of the step. In release, it is what the game really costs:
//!
//! ```text
//! cargo test --release --bin aworld bench_scenes -- --ignored --nocapture
//! ```
//!
//! `AWORLD_BENCH_STEPS` sets how many steps each scene is measured over (300 by default),
//! `AWORLD_BENCH_SCENE` runs only the scene of that name, `AWORLD_BENCH_SERIES=1` also prints
//! the time of the first 96 measured steps one by one (to see the steps that stick out).

use super::*;
use std::time::Instant;

/// The ground of a countryside world, `n` cells below the top of the land at `x`.
fn ground(w: &World, x: i32) -> i32 {
    (0..H() as i32).find(|&y| w.raw(x, y).is_hard() || matches!(w.raw(x, y), Mat::Dirt | Mat::Grass | Mat::Sand | Mat::Water)).unwrap_or(H() as i32)
}

fn countryside() -> World {
    let mut w = World::new();
    w.generate_biome(3, Biome::Countryside);
    w
}

/// The world the game opens on, left to run: grass, trees, sky, weather.
fn demo() -> World {
    countryside()
}

/// Six charges of powder and dynamite going off under the fields at once.
fn explosions() -> World {
    let mut w = countryside();
    for k in 0..6 {
        let x = 120 + k * 140;
        let g = ground(&w, x);
        for dx in -12..12 {
            for dy in 8..24 {
                w.set(x + dx, g + dy, if k % 2 == 0 { Mat::Dynamite } else { Mat::Gunpowder });
            }
        }
        w.set(x, g + 8, Mat::Fire);
        w.set(x, g + 7, Mat::Fire);
    }
    w
}

/// The sea, and a big wall of water poured over the land: a lot of moving liquid.
fn liquids() -> World {
    let mut w = World::new();
    w.generate_biome(3, Biome::Sea);
    for (x, y) in [(200, 120), (300, 110), (420, 130), (560, 120)] {
        w.paint(x, y, 40, Mat::Water);
    }
    w
}

/// Water painted over a big cloud in the open sky (Thomas's scene of 26 October): where the
/// water goes, what the solver takes for a wall, and what a step costs.
///
/// ```text
/// cargo test --release --bin aworld cloud_probe -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn cloud_probe() {
    let mut w = countryside();
    w.set_liquid_physics(true);
    for y in 120..400 {
        for x in 300..900 {
            w.set(x, y, Mat::Cloud);
        }
    }
    for y in 100..120 {
        for x in 350..850 {
            w.set(x, y, Mat::Water);
        }
    }
    w.prof.on = true;
    let dt: f32 = std::env::var("AWORLD_PROBE_DT").ok().and_then(|s| s.parse().ok()).unwrap_or(STEP);
    let mut spent = 0.0;
    let steps: usize = std::env::var("AWORLD_PROBE_STEPS").ok().and_then(|s| s.parse().ok()).unwrap_or(300);
    for k in 0..=steps {
        if k % 60 == 0 {
            let (mut water, mut over_gas, mut walls, mut rain, mut rows, mut still) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
            let (mut top, mut bottom) = (usize::MAX, 0usize);
            for i in 0..W() * (H() - 1) {
                let m = w.cells[i].m;
                if m == Mat::Water {
                    water += 1;
                    rows += i / W();
                    still += (w.vel[i] == 0) as usize;
                    top = top.min(i / W());
                    bottom = bottom.max(i / W());
                    if w.cells[i + W()].m.kind() == Kind::Gas {
                        over_gas += 1;
                    }
                    if w.liquid_class(i % W(), i / W()).is_some_and(|c| c & 3 == super::liquid::AIR) {
                        rain += 1;
                    }
                }
                if m.kind() == Kind::Gas && w.liquid_class(i % W(), i / W()).is_some_and(|c| c & 3 == super::liquid::SOLID) {
                    walls += 1;
                }
            }
            eprintln!(
                "{:.1} s : {water} cellules d'eau (rangs {top}..{bottom}, moyen {}, {still} immobiles) dont {over_gas} sur du gaz et {rain} en pluie, {} gouttes, {walls} cases de gaz prises pour une paroi, {:.1} ms/pas",
                k as f32 * dt,
                rows / water.max(1),
                w.liq.drops.len(),
                if k > 0 { spent * 1000.0 / 60.0 } else { 0.0 }
            );
            spent = 0.0;
        }
        let t = Instant::now();
        w.step_dt(dt);
        spent += t.elapsed().as_secs_f64();
    }
    eprint!("{}", w.prof.report().lines().take(10).map(|l| format!("{l}\n")).collect::<String>());
}

/// A small slab of water over a cloud, tick by tick: the row of each cell of the middle column
/// and its speed.
#[test]
#[ignore]
fn slab_probe() {
    let mut w = World::new_static();
    w.set_liquid_physics(true);
    for y in 60..120 {
        for x in 80..140 {
            w.set(x, y, Mat::Cloud);
        }
    }
    for y in 55..60 {
        for x in 100..120 {
            w.set(x, y, Mat::Water);
        }
    }
    for k in 0..40 {
        let col: Vec<String> = (40..130).filter(|&y| w.raw(110, y) == Mat::Water).map(|y| format!("{y}:{}", w.vel[World::idx(110, y)])).collect();
        let gas_in: usize = (55..60).flat_map(|y| (100..120).map(move |x| (x, y))).filter(|&(x, y)| w.raw(x, y).kind() == Kind::Gas).count();
        eprintln!("tick {k}: {} (gaz dans la dalle {gas_in}, gouttes {})", col.join(" "), w.liq.drops.len());
        w.step();
    }
}

/// A big charged storm cloud over the valley, and a bolt every few steps.
fn storm() -> World {
    let mut w = countryside();
    for x in 280..680 {
        let top = 70 + ((x as f32 * 0.05).sin() * 8.0) as i32;
        for y in top..200 {
            w.set(x, y, Mat::Cloud);
        }
    }
    for qy in 0..QH() {
        for qx in 0..QW() {
            let (x, y) = ((qx * QS) as f32, (qy * QS) as f32);
            if (280.0..680.0).contains(&x) && (70.0..200.0).contains(&y) {
                let u = ((x - 480.0) / 200.0).powi(2);
                w.charge[qy * QW() + qx] = (if y < 120.0 { 0.8 } else if y > 150.0 { -0.95 } else { 0.0 }) * (1.0 - u);
            }
        }
    }
    w
}

/// A forest fire: the trees and the grass of the countryside set alight.
fn fire() -> World {
    let mut w = countryside();
    for x in (20..W() as i32 - 20).step_by(25) {
        let g = ground(&w, x);
        w.paint(x, g - 2, 3, Mat::Fire);
    }
    w
}

pub(super) fn scenes() -> Vec<(&'static str, fn() -> World, Option<fn(&mut World, u32)>)> {
    vec![
        ("démo", demo as fn() -> World, None),
        ("explosions", explosions, None),
        ("liquides", liquids, None),
        ("orage", storm, Some(|w: &mut World, i: u32| {
            if i % 40 == 0 {
                w.strike(300 + (i as i32 * 37) % 360, 0);
            }
        })),
        ("incendie", fire, None),
    ]
}

#[test]
#[ignore]
fn bench_scenes() {
    let steps: u32 = std::env::var("AWORLD_BENCH_STEPS").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
    let only = std::env::var("AWORLD_BENCH_SCENE").ok();
    // The length of a step, in ticks of 1/60 s (AWORLD_BENCH_DT=2: steps of 1/30 s).
    let ticks: f32 = std::env::var("AWORLD_BENCH_DT").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let dt = ticks * super::STEP;
    let steps = (steps as f32 / ticks).round() as u32;
    for (name, make, each) in scenes() {
        if only.as_deref().map_or(false, |o| o != name) {
            continue;
        }
        let mut w = make();
        // A short warm-up so that the first explosions and flows get going.
        for i in 0..30 {
            if let Some(f) = each {
                f(&mut w, i);
            }
            w.step();
        }
        w.prof.on = true;
        w.prof.reset();
        let mut worst = 0f64;
        let mut series = String::new();
        let t = Instant::now();
        for i in 0..steps {
            if let Some(f) = each {
                f(&mut w, i);
            }
            let s = Instant::now();
            w.step_dt(dt);
            let ms = s.elapsed().as_secs_f64() * 1000.0;
            worst = worst.max(ms);
            if i < 96 {
                series += &format!("{ms:5.1}{}", if i % 16 == 15 { "\n" } else { " " });
            }
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / steps as f64;
        eprintln!("\n{name}: {:.0} ms de calcul par seconde du monde (pas de {:.1} ms)", ms * (1.0 / dt) as f64, dt * 1000.0);
        let awake = w.zones.awake.iter().filter(|&&a| a).count();
        eprintln!(
            "\n=== {name}: {ms:.2} ms/pas en moyenne, pire pas {worst:.1} ms, {awake}/{} zones éveillées à la fin",
            w.zones.awake.len()
        );
        if std::env::var_os("AWORLD_BENCH_SERIES").is_some() {
            eprintln!("{series}");
        }
        eprint!("{}", w.prof.report());
    }
}

/// A fingerprint of every scene after a run: the same numbers before and after an optimisation
/// that must not change the simulation.
#[test]
#[ignore]
fn bench_fingerprint() {
    for lod in [0, 1, 3] {
    for (name, make, each) in scenes() {
        if lod > 0 && matches!(name, "explosions" | "incendie") {
            continue;
        }
        let mut w = make();
        w.lod = lod;
        for i in 0..240 {
            if let Some(f) = each {
                f(&mut w, i);
            }
            w.step();
        }
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x100_0000_01b3);
        };
        for i in 0..W() * H() {
            eat(w.cells[i].m as u64);
            eat(w.temp[i].to_bits() as u64);
        }
        eprintln!("FINGERPRINT {name} lod {lod}: {h:016x}");
    }
    }
}

/// The cells' turn on every core comes out the same on one core as on four (see `turn.rs`).
#[test]
fn the_cells_turn_on_every_core_is_the_same_on_one_core() {
    let run = |cores: usize| {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(cores).build().unwrap();
        pool.install(|| {
            let mut w = explosions();
            w.parallel_cells = true;
            for _ in 0..90 {
                w.step();
            }
            (w.cells.iter().map(|c| c.m).collect::<Vec<_>>(), w.temp.iter().map(|t| t.to_bits()).collect::<Vec<_>>())
        })
    };
    assert!(run(1) == run(4), "the turn depends on the number of cores");
}

/// What a scene looks like after a while, in a few numbers that do not depend on the lots drawn:
/// the water (its amount, and the height of its centre), the mean temperature, what burns.
fn summary(w: &World) -> (f64, f64, f64, usize) {
    let (mut water, mut wy, mut temp, mut burning) = (0f64, 0f64, 0f64, 0usize);
    for i in 0..W() * H() {
        let m = w.cells[i].m;
        if m == Mat::Water {
            let f = w.liquid_fill(i) as f64;
            water += f;
            wy += f * (i / W()) as f64;
        }
        if matches!(m, Mat::Fire | Mat::Ember) {
            burning += 1;
        }
        temp += w.temp[i] as f64;
    }
    (water, wy / water.max(1.0), temp / (W() * H()) as f64, burning)
}

/// The same seconds of a scene at steps of 1/120, 1/60, 1/30 and 1/20 s:
///
/// ```text
/// cargo test --release --bin aworld same_seconds_at_any_step -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn same_seconds_at_any_step() {
    let seconds = 3.0;
    for (name, make, _) in scenes() {
        if !matches!(name, "liquides" | "incendie" | "démo") || std::env::var("AWORLD_BENCH_SCENE").is_ok_and(|o| o != name) {
            continue;
        }
        let w0 = make();
        let (_, _, t0, _) = summary(&w0);
        eprintln!("{name:>9} au départ : {t0:.2} °C en moyenne");
        let seeds: u32 = std::env::var("AWORLD_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
        for (ticks, seed) in [0.5f32, 1.0, 2.0, 3.0].into_iter().flat_map(|t| (0..seeds).map(move |s| (t, s))) {
            let mut w = make();
            // (Other lots for the same scene.)
            for _ in 0..seed {
                w.rand();
            }
            let dt = ticks * STEP;
            let t = Instant::now();
            for _ in 0..(seconds / dt).round() as u32 {
                w.step_dt(dt);
            }
            let (water, wy, temp, burning) = summary(&w);
            eprintln!(
                "{name:>9} pas de 1/{:<3.0} s : eau {water:.0} (centre à y = {wy:.1}), {temp:.2} °C en moyenne, {burning} cases en feu, horloge {}, parti {} séché {} ({:.1} s de calcul)",
                1.0 / dt,
                w.clock,
                w.gone,
                w.dried,
                t.elapsed().as_secs_f64()
            );
        }
    }
}

#[test]
#[ignore]
fn water_over_time_probe() {
    for ticks in [0.5f32, 1.0, 2.0] {
        let mut w = liquids();
        let dt = ticks * STEP;
        let mut line = format!("1/{:.0}:", 1.0 / dt);
        for k in 0..=(3.0 / dt) as u32 {
            if k % ((0.5 / dt) as u32) == 0 {
                line += &format!(" {:.4}", summary(&w).0 / 1e9);
            }
            w.step_dt(dt);
        }
        eprintln!("{line}");
    }
}

/// How fast things fall from rest: a block of water, a drop, sand, in cells after each tenth of a
/// second (the probe for the same gravity for all).
#[test]
#[ignore]
fn falls_probe() {
    // The centre of the matter (weighted by its fill for a liquid), and its lowest cell.
    let lowest = |w: &World, m: Mat| {
        let (mut s, mut n, mut low) = (0f64, 0f64, 0);
        for i in (0..W() * H()).filter(|&i| w.cells[i].m == m) {
            let f = if m.kind() == Kind::Liquid { w.liquid_fill(i) as f64 } else { 1.0 };
            s += f * (i / W()) as f64;
            n += f;
            low = low.max(i / W());
        }
        format!("{:.1} (bas {low})", s / n.max(1.0))
    };
    let mut w = World::new_static();
    w.liquid_physics = true;
    for y in 40..50 {
        for x in 100..110 {
            w.set(x, y, Mat::Water);
        }
        for x in 300..310 {
            w.set(x, y, Mat::Sand);
        }
    }
    w.liq.drops.push(super::liquid::Drop { x: 500.5, y: 49.5, vx: 0.0, vy: 0.0, m: Mat::Water, q: field::FULL, t: 15.0 });
    let mut line = String::new();
    for k in 0..=60 {
        if k % 6 == 0 {
            let drop = w.liq.drops.first().map_or(-1.0, |d| d.y);
            line += &format!("t={:.1}s eau {} goutte {:.0} sable {}\n", k as f32 / 60.0, lowest(&w, Mat::Water), drop, lowest(&w, Mat::Sand));
        }
        w.step();
    }
    eprintln!("{line}");
}

/// Everything falls by the same gravity, whatever the length of the steps: a block of sand
/// dropped from rest goes down about ½ g t² (29 cells of 1/6 m in a second, a little less with
/// the drag), and keeps together on the way.
#[test]
fn a_block_of_sand_falls_by_gravity_at_any_step() {
    for ticks in [1.0f32, 2.0, 0.5] {
        let mut w = World::new_static();
        for y in 40..50 {
            for x in 300..310 {
                w.set(x, y, Mat::Sand);
            }
        }
        let dt = ticks * STEP;
        for _ in 0..(1.0 / dt).round() as u32 {
            w.step_dt(dt);
        }
        let ys: Vec<usize> = (0..W() * H()).filter(|&i| w.cells[i].m == Mat::Sand).map(|i| i / W()).collect();
        let (low, high) = (*ys.iter().max().unwrap(), *ys.iter().min().unwrap());
        assert!((24..=31).contains(&(low - 49)), "steps of 1/{:.0} s: the bottom fell {} cells in a second", 1.0 / dt, low - 49);
        assert!(low - high <= 12, "steps of 1/{:.0} s: the block spread over {} rows", 1.0 / dt, low - high + 1);
    }
}
