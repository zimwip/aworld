//! The settings read from the environment (`AWORLD_*`), read once and all listed here (and in the
//! README). The game reads `config()`; the benches and probes of the tests read `bench()`.

use std::sync::OnceLock;

/// Whether the liquids' pressure goes to the graphics card (`AWORLD_GPU`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gpu {
    /// Never (unset, or any other value).
    Off,
    /// When the system is large enough to pay for the trip (`auto`).
    Auto,
    /// Always (`1`).
    Always,
}

/// The settings of the game.
#[derive(Debug)]
pub struct Config {
    /// `AWORLD_GRID=1440x810`: the size of the world, in cells (`--grid` comes first).
    pub grid: Option<String>,
    /// `AWORLD_GPU`: see `Gpu`.
    pub gpu: Gpu,
    /// `AWORLD_GPU_MIN`: how many unknowns a pressure system needs to go to the card.
    pub gpu_min: Option<usize>,
    /// `AWORLD_GPU_FUSE`: the level of the multigrid from which a V-cycle runs in one launch.
    pub gpu_fuse: Option<usize>,
    /// `AWORLD_CELLS_PAR=0`: the cells' turn on one core only.
    pub cells_par: bool,
    /// `AWORLD_PROFILE` (not `0`): the time of each part of a step, logged, and the card timed.
    pub profile: bool,
    /// `AWORLD_DEBUG_MASS`: the liquids check that no liquid is made or lost, step by step.
    pub debug_mass: bool,
    /// `AWORLD_DEBUG_CARD`: what the card does, logged, and its passes timed.
    pub debug_card: bool,
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn number<T: std::str::FromStr>(name: &str) -> Option<T> {
    var(name).and_then(|v| v.parse().ok())
}

/// The settings of the game, read from the environment the first time they are asked for.
pub fn config() -> &'static Config {
    static C: OnceLock<Config> = OnceLock::new();
    C.get_or_init(|| Config {
        grid: var("AWORLD_GRID"),
        gpu: match var("AWORLD_GPU").as_deref() {
            Some("1") => Gpu::Always,
            Some("auto") => Gpu::Auto,
            _ => Gpu::Off,
        },
        gpu_min: number("AWORLD_GPU_MIN"),
        gpu_fuse: number("AWORLD_GPU_FUSE"),
        cells_par: var("AWORLD_CELLS_PAR").is_none_or(|v| v != "0"),
        profile: var("AWORLD_PROFILE").is_some_and(|v| v != "0"),
        debug_mass: var("AWORLD_DEBUG_MASS").is_some(),
        debug_card: var("AWORLD_DEBUG_CARD").is_some(),
    })
}

/// The settings of the benches and probes (tests run with `--ignored`).
#[cfg(test)]
#[derive(Debug)]
pub struct Bench {
    /// `AWORLD_BENCH_STEPS`: steps a bench runs.
    pub steps: Option<u32>,
    /// `AWORLD_BENCH_SCENE`: the one scene to run.
    pub scene: Option<String>,
    /// `AWORLD_BENCH_DT`: the length of a step, in ticks of 1/60 s.
    pub ticks: Option<f32>,
    /// `AWORLD_BENCH_SERIES`: print the time of the first steps one by one.
    pub series: bool,
    /// `AWORLD_BENCH_GRID=WxH`: the grid of the card's bench.
    pub grid: Option<String>,
    /// `AWORLD_FLAME_DIR`: where `flame_probe` writes its graphs.
    pub flame_dir: Option<String>,
    /// `AWORLD_SEEDS`: how many worlds a probe averages over.
    pub seeds: Option<u32>,
    /// `AWORLD_PROBE_DT`, `AWORLD_PROBE_STEPS`: the step and the length of a probe.
    pub probe_dt: Option<f32>,
    pub probe_steps: Option<usize>,
    /// `AWORLD_FLOOD_DIR`: where `flood_probe` writes its pictures.
    pub flood_dir: Option<String>,
    /// `AWORLD_TEST_GRID=WxH`: the grid `grids_probe` measures on.
    pub test_grid: Option<String>,
    /// `AWORLD_SNAP`: where `snapshot_tumbling` writes its frames.
    pub snap: Option<String>,
}

/// The settings of the benches and probes, read once.
#[cfg(test)]
pub fn bench() -> &'static Bench {
    static B: OnceLock<Bench> = OnceLock::new();
    B.get_or_init(|| Bench {
        steps: number("AWORLD_BENCH_STEPS"),
        scene: var("AWORLD_BENCH_SCENE"),
        ticks: number("AWORLD_BENCH_DT"),
        series: var("AWORLD_BENCH_SERIES").is_some(),
        grid: var("AWORLD_BENCH_GRID"),
        flame_dir: var("AWORLD_FLAME_DIR"),
        seeds: number("AWORLD_SEEDS"),
        probe_dt: number("AWORLD_PROBE_DT"),
        probe_steps: number("AWORLD_PROBE_STEPS"),
        flood_dir: var("AWORLD_FLOOD_DIR"),
        test_grid: var("AWORLD_TEST_GRID"),
        snap: var("AWORLD_SNAP"),
    })
}

/// `WxH` (or `W*H`) as two numbers.
pub fn parse_size(s: &str) -> Option<(usize, usize)> {
    let (w, h) = s.split_once(['x', 'X', '*'])?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}
