//! The performance bench: a few busy scenes, each run for a while, with the time of every phase
//! of the step. In release, it is what the game really costs:
//!
//! ```text
//! cargo test --release --bin aworld bench_scenes -- --ignored --nocapture
//! ```
//!
//! `AWORLD_BENCH_STEPS` sets how many steps each scene is measured over (300 by default),
//! `AWORLD_BENCH_SCENE` runs only the scene of that name.

use super::*;
use std::time::Instant;

/// The ground of a countryside world, `n` cells below the top of the land at `x`.
fn ground(w: &World, x: i32) -> i32 {
    (0..H as i32).find(|&y| w.raw(x, y).is_hard() || matches!(w.raw(x, y), Mat::Dirt | Mat::Grass | Mat::Sand | Mat::Water)).unwrap_or(H as i32)
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

/// A big charged storm cloud over the valley, and a bolt every few steps.
fn storm() -> World {
    let mut w = countryside();
    for x in 280..680 {
        let top = 70 + ((x as f32 * 0.05).sin() * 8.0) as i32;
        for y in top..200 {
            w.set(x, y, Mat::Cloud);
        }
    }
    for qy in 0..QH {
        for qx in 0..QW {
            let (x, y) = ((qx * QS) as f32, (qy * QS) as f32);
            if (280.0..680.0).contains(&x) && (70.0..200.0).contains(&y) {
                let u = ((x - 480.0) / 200.0).powi(2);
                w.charge[qy * QW + qx] = (if y < 120.0 { 0.8 } else if y > 150.0 { -0.95 } else { 0.0 }) * (1.0 - u);
            }
        }
    }
    w
}

/// A forest fire: the trees and the grass of the countryside set alight.
fn fire() -> World {
    let mut w = countryside();
    for x in (20..W as i32 - 20).step_by(25) {
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
        let t = Instant::now();
        for i in 0..steps {
            if let Some(f) = each {
                f(&mut w, i);
            }
            let s = Instant::now();
            w.step();
            worst = worst.max(s.elapsed().as_secs_f64() * 1000.0);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / steps as f64;
        let awake = w.zones.awake.iter().filter(|&&a| a).count();
        eprintln!(
            "\n=== {name}: {ms:.2} ms/pas en moyenne, pire pas {worst:.1} ms, {awake}/{} zones éveillées à la fin",
            w.zones.awake.len()
        );
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
        for i in 0..W * H {
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
