//! The units of the world, the same for every model of it: one size of cell, one tick, one
//! gravity. Every model that moves something by a physical law (the liquids, the grains and the
//! drops, the pockets of gas, the bodies that fall, the blasts, the bodies of the building blocks)
//! takes its scale from here, so that a stone and a block of water dropped side by side fall
//! together, and a world of more cells is the same landscape in finer cells, with the same
//! physics.
//!
//! A cell is `REF_CELL_M` across on the design grid (`BASE_W` columns: 48 m across), and finer
//! as the world has more columns (`set_cell`, from `set_grid`): 2.5 cm at 1920 columns, 12.5 cm
//! at 384. The size is set once, before the first world is made, like the size of the grid.

use super::BASE_W;

/// A cell on the design grid, in metres: the scale the boy, the blocks and the cars are drawn at
/// (the boy is 1.4 m tall).
pub const REF_CELL_M: f32 = 0.05;
/// A tick of the cells' turn, in seconds.
pub const TICK: f32 = 1.0 / 60.0;
/// Gravity, m/s².
pub const G: f32 = 9.81;
/// One atmosphere, in pascals.
pub const ATM: f32 = 101_325.0;
/// Air and water, kg/m³.
pub const RHO_AIR: f32 = 1.2;
pub const RHO_WATER: f32 = 1000.0;

/// The size of a cell, metres (see `cell_m`). A plain read, as `W()`: written once by
/// `set_cell` from the main thread before any world exists.
static mut CELL: f32 = REF_CELL_M;

/// Sets the size of a cell for a world `w` columns across: the design grid's landscape over
/// that many columns.
pub fn set_cell(w: usize) {
    // SAFETY: see `CELL`.
    unsafe {
        CELL = REF_CELL_M * BASE_W as f32 / w as f32;
    }
}

/// A cell, in metres.
#[inline(always)]
pub fn cell_m() -> f32 {
    unsafe { CELL }
}

/// How many cells of this grid make a cell of the design grid: the rates tuned per cell of the
/// design grid (a diffusion, a spread) are scaled by it.
#[inline]
pub fn finer() -> f32 {
    REF_CELL_M / cell_m()
}

/// Gravity in cells per second².
#[inline]
pub fn g_cells() -> f32 {
    G / cell_m()
}

/// Gravity in cells per tick².
#[inline]
pub fn g_tick() -> f32 {
    g_cells() * TICK * TICK
}

/// A speed in m/s, in cells per tick.
#[inline]
pub fn cells_per_tick(v: f32) -> f32 {
    v * TICK / cell_m()
}

/// A speed in cells per tick, in m/s.
#[inline]
pub fn metres_per_s(v: f32) -> f32 {
    v * cell_m() / TICK
}

/// How much a column of one cell of matter `rho` times as dense as water weighs, atmospheres.
#[inline]
pub fn atm_per_cell(rho: f32) -> f32 {
    RHO_WATER * rho * G * cell_m() / ATM
}

/// The weight of a cell of water, kN per metre of depth (ρ g h²).
#[inline]
pub fn cell_of_water_kn() -> f32 {
    RHO_WATER * G * cell_m() * cell_m() / 1000.0
}

/// A WGSL source with its constants of scale (`const CELL_M`, `const G`, `const RHO_AIR`) set to
/// the world's: the shaders state them once, for reading, and run with these.
pub fn wgsl(src: &str) -> String {
    let set = |name: &str, v: f32| (format!("const {name}: f32 ="), format!("const {name}: f32 = {v:?};"));
    let lines = [set("CELL_M", cell_m()), set("G", G), set("RHO_AIR", RHO_AIR)];
    src.lines()
        .map(|l| lines.iter().find(|(start, _)| l.starts_with(start.as_str())).map_or(l.to_string(), |(_, with)| with.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_design_grid_has_cells_of_5_cm_and_gravity_of_196_cells_a_second_squared() {
        assert_eq!(cell_m(), REF_CELL_M, "the tests run on the design grid");
        assert!((g_cells() - 196.2).abs() < 0.01);
        assert!((atm_per_cell(1.0) - 1.0 / 206.6).abs() < 1e-5, "10.3 m of water, 206 cells, make an atmosphere");
    }

    #[test]
    fn the_shaders_take_the_scale_of_the_world() {
        for src in [include_str!("liquid.wgsl")] {
            let out = wgsl(src);
            assert!(out.contains(&format!("const CELL_M: f32 = {:?};", cell_m())), "the liquids' shader states CELL_M");
            assert!(out.contains(&format!("const G: f32 = {G:?};")));
            assert!(out.contains(&format!("const RHO_AIR: f32 = {RHO_AIR:?};")));
        }
    }
}
