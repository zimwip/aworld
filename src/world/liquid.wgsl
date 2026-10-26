// The liquids' turn on the graphics card (see liquid_gpu.rs; liquid.rs is the same turn on the
// CPU, and says why each rule is what it is). Every kernel is one cell (or one edge) a thread,
// over the band of rows the step works on; the pressure's system is built here into the grids
// of the solver (mgpcg.wgsl), which solves it in place.
//
// The cells: `code` is the material (bits 0-7) and the flags of each cell (masked by a block,
// gas open to a pocket, zone awake), `fill` the units of liquid (FULL a full cell), `temp` its
// heat, `gaspa` the pressure of the gas over the ambient one (pockets). What the kernels make:
// `class` (SOLID, AIR, WET, FULLISH, + 4 when the zone runs), `p`, the velocities `u` (W + 1 a
// row) and `v` (W a row, H + 1 rows).

struct Params {
    w: u32,
    h: u32,
    y0: u32,
    rows: u32,
    dt: f32,
    colour: u32,
    patches: u32,
    groups: u32,
    // The step's stamp (a cell is reported changed once a step), and the cells sent this step.
    stamp: u32,
    sent: u32,
    // The game's clock, s (the swells of the wind's pressure run with it).
    time: f32,
    pad3: u32,
}

// A cell as the CPU sends it (what the rest of the game changed), and as the card reports it.
struct CellRecord {
    cell: u32,
    code: u32,
    fill: u32,
    temp: f32,
    gaspa: f32,
}

struct Patch {
    // The edge (bit 31 set: a downward edge, in v), how (0: at least `value`; 1: `weight` of
    // the way to `value`), the value, the weight.
    edge: u32,
    mode: u32,
    value: f32,
    weight: f32,
}

// A drop thrown off: the cell, the units, where it starts, its velocity, its material, its heat.
struct Throw {
    cell: u32,
    q: u32,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    mat: u32,
    temp: f32,
}

@group(0) @binding(0) var<uniform> P: Params;
// The solver's finest level (mgpcg.wgsl, group 1 there).
@group(0) @binding(1) var<storage, read_write> kind: array<u32>;
@group(0) @binding(2) var<storage, read_write> ax: array<f32>;
@group(0) @binding(3) var<storage, read_write> ay: array<f32>;
@group(0) @binding(4) var<storage, read_write> diag: array<f32>;
@group(0) @binding(5) var<storage, read_write> b: array<f32>;
@group(0) @binding(6) var<storage, read_write> x: array<f32>;

@group(1) @binding(0) var<storage, read_write> code: array<u32>;
@group(1) @binding(1) var<storage, read_write> fill: array<u32>;
@group(1) @binding(2) var<storage, read_write> temp: array<f32>;
@group(1) @binding(3) var<storage, read_write> gaspa: array<f32>;
@group(1) @binding(4) var<storage, read_write> cls: array<u32>;
@group(1) @binding(5) var<storage, read_write> p: array<f32>;
@group(1) @binding(6) var<storage, read_write> nu: array<f32>;
@group(1) @binding(7) var<storage, read_write> rhs: array<f32>;
// The materials: kind, density (kg/m³), viscosity (m²/s) and flags (1: lava) of each, 64 apiece.
@group(1) @binding(8) var<storage, read_write> tables: array<u32>;

@group(2) @binding(0) var<storage, read_write> want: array<u32>;
@group(2) @binding(1) var<storage, read_write> scale: array<f32>;
@group(2) @binding(2) var<storage, read_write> qnew: array<i32>;
@group(2) @binding(3) var<storage, read_write> tnew: array<f32>;
@group(2) @binding(4) var<storage, read_write> fu: array<i32>;
@group(2) @binding(5) var<storage, read_write> fv: array<i32>;
@group(2) @binding(6) var<storage, read_write> thrown: array<u32>;
@group(2) @binding(7) var<storage, read_write> mnew: array<u32>;
@group(2) @binding(8) var<storage, read_write> partials: array<f32>;
// The stamp of the step a cell was last reported changed in.
@group(2) @binding(9) var<storage, read_write> mark: array<atomic<u32>>;

@group(3) @binding(0) var<storage, read_write> u: array<f32>;
@group(3) @binding(1) var<storage, read_write> v: array<f32>;
@group(3) @binding(2) var<storage, read_write> u2: array<f32>;
@group(3) @binding(3) var<storage, read_write> v2: array<f32>;
@group(3) @binding(4) var<storage, read_write> u3: array<f32>;
@group(3) @binding(5) var<storage, read_write> v3: array<f32>;
@group(3) @binding(6) var<storage, read_write> patches: array<Patch>;
// The drops thrown this step: the count, then the records.
@group(3) @binding(7) var<storage, read_write> throws_n: atomic<u32>;
@group(3) @binding(8) var<storage, read_write> throws: array<Throw>;
// The cells the CPU sends, the cells the card changed (a count, then their indices), and
// what they are now for the CPU.
@group(3) @binding(9) var<storage, read_write> sent: array<CellRecord>;

const WG: u32 = 256u;
const FULL: u32 = 65536u;
const HOLDS: u32 = 2u;
const CELL_M: f32 = 1.0 / 6.0;
const G: f32 = 9.81;
const REST: f32 = 0.01;
const REFILL: f32 = 0.1;
const SPRAY: f32 = 9.0;
const CALM: f32 = 3.0;
const LAUNCH: f32 = 4.43;
const CRUMB: u32 = 8u;
const THROW: f32 = 2.0;
const THROW_SHARE: f32 = 0.5;
const MAX_THROWS: u32 = 16384u;
// The wind on the liquids (liquid.rs: RHO_AIR, SKIN, GUST, SWELLS, flow.rs: CELL, STIR).
const RHO_AIR: f32 = 1.2;
const SKIN: f32 = 0.25;
const GUST: f32 = 10.0;
const FLOW_CELL: u32 = 8u;
const STIR: f32 = 1.5;
// Where the wind over the surface of the liquids starts in `tables` (flow.rs: `skim`).
const SKIM: u32 = 256u;

const SOLID: u32 = 0u;
const AIR: u32 = 1u;
const WET: u32 = 2u;
const FULLISH: u32 = 3u;
const RUNS: u32 = 4u;

// Materials (as `Mat` in world.rs) and kinds (as `Kind`).
const MAT_EMPTY: u32 = 0u;
const MAT_WATER: u32 = 2u;
const MAT_STONE: u32 = 7u;
const K_EMPTY: u32 = 0u;
const K_STATIC: u32 = 1u;
const K_LIQUID: u32 = 3u;
const K_GAS: u32 = 4u;

const MASKED: u32 = 0x100u;
const OPEN: u32 = 0x200u;
const AWAKE: u32 = 0x400u;

// The solver's kinds.
const S_NONE: u32 = 0u;
const S_FLUID: u32 = 1u;
const S_FIXED: u32 = 2u;

fn mat_of(c: u32) -> u32 {
    return code[c] & 0xffu;
}
fn mat_kind(m: u32) -> u32 {
    return tables[m];
}
fn density(m: u32) -> f32 {
    return bitcast<f32>(tables[64u + m]);
}
fn mat_nu(m: u32) -> f32 {
    return bitcast<f32>(tables[128u + m]);
}
fn is_lava(m: u32) -> bool {
    return (tables[192u + m] & 1u) != 0u;
}
/// The kind of the cell as the world sees it (a block over it is static; outside, stone over
/// the top and nothing under or beside).
fn at_kind(xi: i32, yi: i32) -> u32 {
    if xi < 0 || yi < 0 || xi >= i32(P.w) || yi >= i32(P.h) {
        if yi < 0 { return K_STATIC; }
        return K_EMPTY;
    }
    let c = u32(yi) * P.w + u32(xi);
    if (code[c] & MASKED) != 0u { return K_STATIC; }
    return mat_kind(code[c] & 0xffu);
}
/// The kind of the cell's own material (what the cells hold, blocks aside).
fn raw_kind(c: u32) -> u32 {
    return mat_kind(code[c] & 0xffu);
}
fn ui(xx: u32, yy: u32) -> u32 {
    return yy * (P.w + 1u) + xx;
}
fn vi(xx: u32, yy: u32) -> u32 {
    return yy * P.w + xx;
}
/// The class of a cell, SOLID outside the world.
fn kind_at(xi: i32, yi: i32) -> u32 {
    if xi < 0 || yi < 0 || xi >= i32(P.w) || yi >= i32(P.h) { return SOLID; }
    return cls[u32(yi) * P.w + u32(xi)] & 3u;
}
fn runs_at(xi: i32, yi: i32) -> bool {
    if xi < 0 || yi < 0 || xi >= i32(P.w) || yi >= i32(P.h) { return false; }
    return (cls[u32(yi) * P.w + u32(xi)] & RUNS) != 0u;
}
fn rho(c: u32) -> f32 {
    return density(mat_of(c)) * 1000.0;
}

/// The wind along x over the liquid at (xx, yy), m/s: none under a roof (flow.rs: `skim_at`).
fn skim_at(xx: u32, yy: u32) -> f32 {
    let fw = P.w / FLOW_CELL;
    let fh = (P.h + FLOW_CELL - 1u) / FLOW_CELL;
    // Under a roof, none.
    if f32(yy) >= bitcast<f32>(tables[SKIM + fw * fh + xx]) { return 0.0; }
    let fx = (f32(xx) + 0.5) / f32(FLOW_CELL) - 0.5;
    let i0 = floor(fx);
    let t = fx - i0;
    let ia = u32((i32(i0) + i32(fw)) % i32(fw));
    let ib = u32((i32(i0) + 1 + i32(fw)) % i32(fw));
    let row = min(yy / FLOW_CELL, fh - 1u) * fw;
    let a = bitcast<f32>(tables[SKIM + row + ia]);
    let bb = bitcast<f32>(tables[SKIM + row + ib]);
    return a + (bb - a) * t;
}
/// The drag of the wind on a rightward edge of the free surface, m/s² (liquid.rs: `wind_push`).
fn wind_push(wind: f32, uu: f32, r: f32) -> f32 {
    let rel = wind - uu;
    if rel * wind <= 0.0 { return 0.0; }
    return SKIN * RHO_AIR * rel * abs(rel) / (r * CELL_M);
}
/// The pressure of the wind on the liquid at column xx, Pa (liquid.rs: `wind_pa`).
fn wind_pa(wind: f32, xx: f32) -> f32 {
    if wind == 0.0 { return 0.0; }
    let sum = 0.5 * swell(wind, xx, 2.0) + 0.3 * swell(wind, xx, 3.5) + 0.2 * swell(wind, xx, 6.0);
    return GUST * RHO_AIR * wind * wind * sum;
}
/// One swell of the wind's pressure, `length` m long, running downwind at the waves' speed.
fn swell(wind: f32, xx: f32, length: f32) -> f32 {
    let kk = 6.2831855 / length;
    return sin(kk * (xx * CELL_M - sign(wind) * sqrt(G / kk) * P.time) + length);
}

/// The cell of a thread of a kernel over the band, or the count if it is past it.
fn band_cell(id: u32) -> u32 {
    let c = id + P.y0 * P.w;
    if c >= (P.y0 + P.rows) * P.w { return 0xffffffffu; }
    return c;
}

/// Notes that cell c changed this step, for the CPU (which reads the marks of the band back).
fn note_change(c: u32) {
    atomicMax(&mark[c], P.stamp);
}

// ---- What the CPU sends: the cells the rest of the game changed.

@compute @workgroup_size(256)
fn apply_cells(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= P.sent { return; }
    let r = sent[id.x];
    code[r.cell] = r.code;
    fill[r.cell] = r.fill;
    temp[r.cell] = r.temp;
    gaspa[r.cell] = r.gaspa;
}

// ---- The patches a step starts with (drops that landed, cells that fell).

@compute @workgroup_size(256)
fn apply_patches(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= P.patches { return; }
    let pt = patches[id.x];
    let e = pt.edge & 0x7fffffffu;
    if (pt.edge & 0x80000000u) != 0u {
        if pt.mode == 0u { v[e] = max(v[e], pt.value); } else { v[e] += pt.weight * (pt.value - v[e]); }
    } else {
        if pt.mode == 0u { u[e] = max(u[e], pt.value); } else { u[e] += pt.weight * (pt.value - u[e]); }
    }
}

// ---- The classes.

@compute @workgroup_size(256)
fn classify(@builtin(global_invocation_id) id: vec3<u32>) {
    let c = band_cell(id.x);
    if c == 0xffffffffu { return; }
    let xx = c % P.w;
    let yy = c / P.w;
    let cd = code[c];
    let m = cd & 0xffu;
    var what = SOLID;
    if (cd & MASKED) == 0u {
        let k = mat_kind(m);
        if k == K_EMPTY {
            what = AIR;
        } else if k == K_GAS {
            if (cd & OPEN) != 0u { what = AIR; } else { what = SOLID; }
        } else if k == K_LIQUID {
            let xi = i32(xx);
            let yi = i32(yy);
            let under = yy + 1u < P.h && (raw_kind(c + P.w) == K_EMPTY || raw_kind(c + P.w) == K_GAS);
            let lone = at_kind(xi - 1, yi) != K_LIQUID && at_kind(xi + 1, yi) != K_LIQUID && at_kind(xi, yi - 1) != K_LIQUID;
            // (Water over a gas that is not open air at its pocket's pressure, a cloud, falls
            // through it as the cells' rain whether it is alone or not: to the liquids the gas
            // is a wall, and the water would lie on the cloud.)
            let on_gas = yy + 1u < P.h && raw_kind(c + P.w) == K_GAS && (code[c + P.w] & OPEN) == 0u;
            if m == MAT_WATER && ((under && lone) || on_gas) {
                what = AIR;
            } else if fill[c] >= FULL / HOLDS {
                what = FULLISH;
            } else {
                what = WET;
            }
        }
    }
    var run = 0u;
    if (cd & AWAKE) != 0u { run = RUNS; }
    cls[c] = what | run;
}

// ---- The forces: the velocities carried along the flow (MacCormack), gravity, the drag.

fn sample_u(px: f32, py: f32) -> f32 {
    let w = P.w + 1u;
    let sx = clamp(px, 0.0, f32(w - 1u) - 0.001);
    let sy = clamp(py, 0.0, f32(P.h - 1u) - 0.001);
    let x0 = u32(sx);
    let y0 = u32(sy);
    let fx = sx - f32(x0);
    let fy = sy - f32(y0);
    return u[y0 * w + x0] * (1.0 - fx) * (1.0 - fy) + u[y0 * w + x0 + 1u] * fx * (1.0 - fy) + u[(y0 + 1u) * w + x0] * (1.0 - fx) * fy + u[(y0 + 1u) * w + x0 + 1u] * fx * fy;
}
fn sample_u2(px: f32, py: f32) -> f32 {
    let w = P.w + 1u;
    let sx = clamp(px, 0.0, f32(w - 1u) - 0.001);
    let sy = clamp(py, 0.0, f32(P.h - 1u) - 0.001);
    let x0 = u32(sx);
    let y0 = u32(sy);
    let fx = sx - f32(x0);
    let fy = sy - f32(y0);
    return u2[y0 * w + x0] * (1.0 - fx) * (1.0 - fy) + u2[y0 * w + x0 + 1u] * fx * (1.0 - fy) + u2[(y0 + 1u) * w + x0] * (1.0 - fx) * fy + u2[(y0 + 1u) * w + x0 + 1u] * fx * fy;
}
fn around_u(px: f32, py: f32) -> vec2<f32> {
    let w = P.w + 1u;
    let sx = clamp(px, 0.0, f32(w - 1u) - 0.001);
    let sy = clamp(py, 0.0, f32(P.h - 1u) - 0.001);
    let x0 = u32(sx);
    let y0 = u32(sy);
    let a = u[y0 * w + x0];
    let bb = u[y0 * w + x0 + 1u];
    let cc = u[(y0 + 1u) * w + x0];
    let d = u[(y0 + 1u) * w + x0 + 1u];
    return vec2<f32>(min(min(a, bb), min(cc, d)), max(max(a, bb), max(cc, d)));
}
fn sample_v(px: f32, py: f32) -> f32 {
    let w = P.w;
    let sx = clamp(px, 0.0, f32(w - 1u) - 0.001);
    let sy = clamp(py, 0.0, f32(P.h) - 0.001);
    let x0 = u32(sx);
    let y0 = u32(sy);
    let fx = sx - f32(x0);
    let fy = sy - f32(y0);
    return v[y0 * w + x0] * (1.0 - fx) * (1.0 - fy) + v[y0 * w + x0 + 1u] * fx * (1.0 - fy) + v[(y0 + 1u) * w + x0] * (1.0 - fx) * fy + v[(y0 + 1u) * w + x0 + 1u] * fx * fy;
}
fn sample_v2(px: f32, py: f32) -> f32 {
    let w = P.w;
    let sx = clamp(px, 0.0, f32(w - 1u) - 0.001);
    let sy = clamp(py, 0.0, f32(P.h) - 0.001);
    let x0 = u32(sx);
    let y0 = u32(sy);
    let fx = sx - f32(x0);
    let fy = sy - f32(y0);
    return v2[y0 * w + x0] * (1.0 - fx) * (1.0 - fy) + v2[y0 * w + x0 + 1u] * fx * (1.0 - fy) + v2[(y0 + 1u) * w + x0] * (1.0 - fx) * fy + v2[(y0 + 1u) * w + x0 + 1u] * fx * fy;
}
fn around_v(px: f32, py: f32) -> vec2<f32> {
    let w = P.w;
    let sx = clamp(px, 0.0, f32(w - 1u) - 0.001);
    let sy = clamp(py, 0.0, f32(P.h) - 0.001);
    let x0 = u32(sx);
    let y0 = u32(sy);
    let a = v[y0 * w + x0];
    let bb = v[y0 * w + x0 + 1u];
    let cc = v[(y0 + 1u) * w + x0];
    let d = v[(y0 + 1u) * w + x0 + 1u];
    return vec2<f32>(min(min(a, bb), min(cc, d)), max(max(a, bb), max(cc, d)));
}

/// The velocity carried on the rightward edge (xx, yy): (1, vx, vy, 0) when it is carried, else
/// (0, keep, 0, 0) for the value it takes.
fn u_vel(xx: u32, yy: u32) -> vec4<f32> {
    let xi = i32(xx);
    let yi = i32(yy);
    let a = kind_at(xi - 1, yi);
    let bb = kind_at(xi, yi);
    let here = u[ui(xx, yy)];
    if a == SOLID || bb == SOLID {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let wet = a >= WET || bb >= WET;
    if !wet || !(runs_at(xi - 1, yi) || runs_at(xi, yi)) {
        if wet { return vec4<f32>(0.0, here, 0.0, 0.0); }
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let vy = 0.25 * (v[vi(xx - 1u, yy)] + v[vi(xx, yy)] + v[vi(xx - 1u, yy + 1u)] + v[vi(xx, yy + 1u)]);
    return vec4<f32>(1.0, here, vy, 0.0);
}
fn v_vel(xx: u32, yy: u32) -> vec4<f32> {
    let xi = i32(xx);
    let yi = i32(yy);
    let a = kind_at(xi, yi - 1);
    let bb = kind_at(xi, yi);
    let here = v[vi(xx, yy)];
    if a == SOLID || bb == SOLID {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let wet = a >= WET || bb >= WET;
    if !wet || !(runs_at(xi, yi - 1) || runs_at(xi, yi)) {
        if wet { return vec4<f32>(0.0, here, 0.0, 0.0); }
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let vx = 0.25 * (u[ui(xx, yy - 1u)] + u[ui(xx + 1u, yy - 1u)] + u[ui(xx, yy)] + u[ui(xx + 1u, yy)]);
    return vec4<f32>(1.0, vx, here, 0.0);
}

/// Carried forward: u2 from u (one thread an edge of the band's rows, W + 1 a row).
@compute @workgroup_size(256)
fn forward_u(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = P.w + 1u;
    let e = id.x + P.y0 * w;
    if e >= (P.y0 + P.rows) * w { return; }
    let xx = e % w;
    let yy = e / w;
    let r = u_vel(xx, yy);
    let step = P.dt / CELL_M;
    if r.x != 0.0 {
        u2[e] = sample_u(f32(xx) - r.y * step, f32(yy) - r.z * step);
    } else {
        u2[e] = r.y;
    }
}
@compute @workgroup_size(256)
fn forward_v(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = P.w;
    let e = id.x + P.y0 * w;
    if e >= (P.y0 + P.rows) * w { return; }
    let xx = e % w;
    let yy = e / w;
    let r = v_vel(xx, yy);
    let step = P.dt / CELL_M;
    if r.x != 0.0 {
        v2[e] = sample_v(f32(xx) - r.y * step, f32(yy) - r.z * step);
    } else {
        v2[e] = r.y;
    }
}
/// Carried back, the gap corrected, the drag of the air: u3 from u and u2.
@compute @workgroup_size(256)
fn back_u(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = P.w + 1u;
    let e = id.x + P.y0 * w;
    if e >= (P.y0 + P.rows) * w { return; }
    let xx = e % w;
    let yy = e / w;
    let r = u_vel(xx, yy);
    if r.x == 0.0 {
        u3[e] = u2[e];
        return;
    }
    let step = P.dt / CELL_M;
    let px = f32(xx) - r.y * step;
    let py = f32(yy) - r.z * step;
    let back = sample_u2(f32(xx) + r.y * step, f32(yy) + r.z * step);
    let lh = around_u(px, py);
    var to = clamp(u2[e] + 0.5 * (u[e] - back), lh.x, lh.y);
    let a = kind_at(i32(xx) - 1, i32(yy));
    let bb = kind_at(i32(xx), i32(yy));
    if a != bb { to = to / (1.0 + CALM * P.dt); }
    // The wind over a free surface drags it along.
    let ua = kind_at(i32(xx) - 1, i32(yy) - 1);
    let ub = kind_at(i32(xx), i32(yy) - 1);
    if a == AIR || bb == AIR || ua == AIR || ua == WET || ub == AIR || ub == WET {
        var c = yy * P.w + xx - 1u;
        if bb >= WET { c = yy * P.w + xx; }
        to += P.dt * wind_push(skim_at(min(xx, P.w - 1u), yy), to, rho(c));
    }
    if a != FULLISH && bb != FULLISH {
        u3[e] = clamp(to, -SPRAY, SPRAY);
    } else {
        u3[e] = to;
    }
}
@compute @workgroup_size(256)
fn back_v(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = P.w;
    let e = id.x + P.y0 * w;
    if e >= (P.y0 + P.rows) * w { return; }
    let xx = e % w;
    let yy = e / w;
    let r = v_vel(xx, yy);
    if r.x == 0.0 {
        v3[e] = v2[e];
        return;
    }
    let step = P.dt / CELL_M;
    let px = f32(xx) - r.y * step;
    let py = f32(yy) - r.z * step;
    let back = sample_v2(f32(xx) + r.y * step, f32(yy) + r.z * step);
    let lh = around_v(px, py);
    let xi = i32(xx);
    let yi = i32(yy);
    let a = kind_at(xi, yi - 1);
    let bb = kind_at(xi, yi);
    var was = clamp(v2[e] + 0.5 * (v[e] - back), lh.x, lh.y);
    // (Not a surface that falls: liquid over air, or liquid under air with air under it.)
    let falls = (a >= WET && bb == AIR) || (a == AIR && bb >= WET && kind_at(xi, yi + 1) == AIR);
    if a != bb && !falls { was = was / (1.0 + CALM * P.dt); }
    let fall = was + G * P.dt;
    if a != FULLISH && bb != FULLISH {
        let f = clamp(fall, -LAUNCH, SPRAY);
        let under = kind_at(xi, yi + 1);
        if a < WET && bb == WET && (under == SOLID || under == FULLISH) {
            v3[e] = min(f, max(v[vi(xx, yy + 1u)], 0.0));
        } else {
            v3[e] = f;
        }
    } else {
        v3[e] = fall;
    }
}

// ---- The viscosity of thick liquids.

/// How thick each cell is (0: not thick).
@compute @workgroup_size(256)
fn thickness(@builtin(global_invocation_id) id: vec3<u32>) {
    let c = band_cell(id.x);
    if c == 0xffffffffu { return; }
    nu[c] = 0.0;
    let cl = cls[c];
    if (cl & 3u) < WET || (cl & RUNS) == 0u { return; }
    let m = mat_of(c);
    let t = temp[c];
    let r = rho(c);
    var n = mat_nu(m);
    var tau = 0.0;
    if is_lava(m) {
        let cool = clamp((1150.0 - t) / 60.0, 0.0, 8.0);
        n = min(exp(cool), 50.0);
        tau = 500.0 * exp(clamp((1150.0 - t) / 150.0, 0.0, 8.0));
    }
    if n * P.dt / (CELL_M * CELL_M) < 0.01 { return; }
    if tau > 0.0 {
        let xx = c % P.w;
        let yy = c / P.w;
        let ux = (u[ui(xx + 1u, yy)] - u[ui(xx, yy)]) / CELL_M;
        let vy = (v[vi(xx, yy + 1u)] - v[vi(xx, yy)]) / CELL_M;
        let yd = min(yy + 1u, P.h - 1u);
        let yu = select(yy - 1u, 0u, yy == 0u);
        let xr = min(xx + 1u, P.w - 1u);
        let xl = select(xx - 1u, 0u, xx == 0u);
        let uy = (0.5 * (u[ui(xx, yd)] + u[ui(xx + 1u, yd)]) - 0.5 * (u[ui(xx, yu)] + u[ui(xx + 1u, yu)])) / (2.0 * CELL_M);
        let vx = (0.5 * (v[vi(xr, yy)] + v[vi(xr, yy + 1u)]) - 0.5 * (v[vi(xl, yy)] + v[vi(xl, yy + 1u)])) / (2.0 * CELL_M);
        let rate = sqrt(2.0 * (ux * ux + vy * vy) + (uy + vx) * (uy + vx));
        n += tau / (r * (rate + 1e-3));
    }
    nu[c] = min(n, 50.0);
}
fn cell_nu(xi: i32, yi: i32) -> f32 {
    if xi < 0 || yi < 0 || xi >= i32(P.w) || yi >= i32(P.h) { return 0.0; }
    return nu[u32(yi) * P.w + u32(xi)];
}
/// One red-black sweep of u - dt ν ∇²u = u_before on the rightward edges (u2 holds u_before).
@compute @workgroup_size(256)
fn viscous_u(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = P.w + 1u;
    let e = id.x + P.y0 * w;
    if e >= (P.y0 + P.rows) * w { return; }
    let xx = e % w;
    let yy = e / w;
    if xx == 0u || xx >= P.w || (xx + yy) % 2u != P.colour { return; }
    let xi = i32(xx);
    let yi = i32(yy);
    let n = 0.5 * (cell_nu(xi - 1, yi) + cell_nu(xi, yi));
    if n == 0.0 { return; }
    let c = P.dt / (CELL_M * CELL_M);
    var sum = 0.0;
    var d = 1.0;
    // Left and right.
    let nl = cell_nu(xi - 1, yi);
    if xx - 1u == 0u { d += c * nl; } else { sum += c * nl * u[ui(xx - 1u, yy)]; d += c * nl; }
    let nr = cell_nu(xi, yi);
    if xx + 1u >= P.w { d += c * nr; } else { sum += c * nr * u[ui(xx + 1u, yy)]; d += c * nr; }
    // Over and under.
    for (var k = 0; k < 2; k++) {
        let ny = select(yi - 1, yi + 1, k == 1);
        let nn = 0.25 * (cell_nu(xi - 1, yi) + cell_nu(xi, yi) + cell_nu(xi - 1, ny) + cell_nu(xi, ny));
        var wall = ny < 0 || ny >= i32(P.h);
        if !wall {
            let row = u32(ny) * P.w;
            wall = (cls[row + xx - 1u] & 3u) == SOLID || (cls[row + xx] & 3u) == SOLID;
        }
        if wall {
            d += c * n;
        } else if nn > 0.0 {
            sum += c * nn * u[ui(xx, u32(ny))];
            d += c * nn;
        }
    }
    u[e] = (u2[e] + sum) / d;
}
@compute @workgroup_size(256)
fn viscous_v(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = P.w;
    let e = id.x + P.y0 * w;
    if e >= (P.y0 + P.rows) * w { return; }
    let xx = e % w;
    let yy = e / w;
    if yy == 0u || yy >= P.h || (xx + yy) % 2u != P.colour { return; }
    let xi = i32(xx);
    let yi = i32(yy);
    let n = 0.5 * (cell_nu(xi, yi - 1) + cell_nu(xi, yi));
    if n == 0.0 { return; }
    let c = P.dt / (CELL_M * CELL_M);
    var sum = 0.0;
    var d = 1.0;
    let nu_ = cell_nu(xi, yi - 1);
    if yy - 1u == 0u { d += c * nu_; } else { sum += c * nu_ * v[vi(xx, yy - 1u)]; d += c * nu_; }
    let nd = cell_nu(xi, yi);
    if yy + 1u >= P.h { d += c * nd; } else { sum += c * nd * v[vi(xx, yy + 1u)]; d += c * nd; }
    for (var k = 0; k < 2; k++) {
        let nx = select(xi - 1, xi + 1, k == 1);
        let nn = 0.25 * (cell_nu(xi, yi - 1) + cell_nu(xi, yi) + cell_nu(nx, yi - 1) + cell_nu(nx, yi));
        var wall = nx < 0 || nx >= i32(P.w);
        if !wall {
            wall = (cls[(yy - 1u) * P.w + u32(nx)] & 3u) == SOLID || (cls[yy * P.w + u32(nx)] & 3u) == SOLID;
        }
        if wall {
            d += c * n;
        } else if nn > 0.0 {
            sum += c * nn * v[vi(u32(nx), yy)];
            d += c * nn;
        }
    }
    v[e] = (v2[e] + sum) / d;
}

// ---- The pressure: the system built into the solver's grids, the solution taken off the
// velocities.

/// The pressure of the gas that stands on the liquid at (xx, yy): the gas there, or over the
/// film of liquid there.
fn gas_pa_over(xi: i32, yi: i32) -> f32 {
    return gas_over(xi, yi) + wind_pa(skim_at(u32(xi), u32(yi)), f32(xi));
}
fn gas_over(xi: i32, yi: i32) -> f32 {
    for (var k = 0; k < 4; k++) {
        let ky = yi - k;
        let kk = at_kind(xi, ky);
        if kk != K_LIQUID {
            if k > 0 && !(kk == K_EMPTY || kk == K_GAS) { return 0.0; }
            if ky < 0 { return 0.0; }
            return gaspa[u32(ky) * P.w + u32(xi)];
        }
    }
    return 0.0;
}

/// The divergence of each full cell that runs (the right side, times dx / dt), and its kind
/// to the solver.
@compute @workgroup_size(256)
fn system_cells(@builtin(global_invocation_id) id: vec3<u32>) {
    let c = band_cell(id.x);
    if c == 0xffffffffu { return; }
    let cl = cls[c];
    if cl == (FULLISH | RUNS) {
        kind[c] = S_FLUID;
    } else if (cl & 3u) == SOLID {
        kind[c] = S_NONE;
    } else {
        kind[c] = S_FIXED;
    }
    if cl != (FULLISH | RUNS) {
        rhs[c] = 0.0;
        return;
    }
    let xx = c % P.w;
    let yy = c / P.w;
    let xi = i32(xx);
    let yi = i32(yy);
    var inside = true;
    for (var k = 0; k < 4; k++) {
        var nx = xi;
        var ny = yi;
        if k == 0 { nx -= 1; } else if k == 1 { nx += 1; } else if k == 2 { ny -= 1; } else { ny += 1; }
        if nx < 0 || ny < 0 || nx >= i32(P.w) || ny >= i32(P.h) { continue; }
        let kk = cls[u32(ny) * P.w + u32(nx)] & 3u;
        if !(kk == SOLID || kk == FULLISH) { inside = false; }
    }
    let kf = CELL_M / P.dt;
    var fix = 0.0;
    if inside {
        fix = REFILL * clamp(f32(fill[c]) / f32(FULL) - 1.0, -1.0, 1.0);
    }
    rhs[c] = (u[ui(xx + 1u, yy)] - u[ui(xx, yy)] + v[vi(xx, yy + 1u)] - v[vi(xx, yy)]) * kf - fix * CELL_M / P.dt * kf;
}

/// The conductance of the edge between cells a and b, 0 unless one of them is an unknown.
fn cond(a: u32, bb: u32) -> f32 {
    let ka = kind[a];
    let kb = kind[bb];
    if ka == S_NONE || kb == S_NONE || (ka != S_FLUID && kb != S_FLUID) { return 0.0; }
    if (cls[a] & 3u) == FULLISH && (cls[bb] & 3u) == FULLISH { return 2.0 / (rho(a) + rho(bb)); }
    if ka == S_FLUID { return 1.0 / rho(a); }
    return 1.0 / rho(bb);
}

/// The edges of the system: a cell's left edge (ax) and top edge (ay).
@compute @workgroup_size(256)
fn system_edges(@builtin(global_invocation_id) id: vec3<u32>) {
    let c = band_cell(id.x);
    if c == 0xffffffffu { return; }
    let xx = c % P.w;
    let yy = c / P.w;
    if xx == 0u {
        ax[ui(0u, yy)] = 0.0;
        ax[ui(P.w, yy)] = 0.0;
    } else {
        ax[ui(xx, yy)] = cond(c - 1u, c);
    }
    if yy == 0u {
        ay[c] = 0.0;
    } else {
        ay[c] = cond(c - P.w, c);
    }
}

/// The diagonal, the right side with the fixed pressures round the cell, and the start of the
/// solution (the last pressure). A full cell walled in on all sides is nothing to the solver.
@compute @workgroup_size(256)
fn assemble(@builtin(global_invocation_id) id: vec3<u32>) {
    let c = band_cell(id.x);
    if c == 0xffffffffu { return; }
    if kind[c] != S_FLUID {
        diag[c] = 0.0;
        b[c] = 0.0;
        x[c] = 0.0;
        return;
    }
    let xx = c % P.w;
    let yy = c / P.w;
    var d = 0.0;
    var r = -rhs[c];
    for (var k = 0; k < 4; k++) {
        var a = 0.0;
        var j = 0u;
        var inside = false;
        if k == 0 { a = ax[ui(xx, yy)]; j = c - 1u; inside = xx > 0u; }
        else if k == 1 { a = ax[ui(xx + 1u, yy)]; j = c + 1u; inside = xx + 1u < P.w; }
        else if k == 2 { a = ay[c]; j = c - P.w; inside = yy > 0u; }
        else { a = ay[c + P.w]; j = c + P.w; inside = yy + 1u < P.h; }
        if a == 0.0 || !inside { continue; }
        d += a;
        if kind[j] == S_FIXED {
            var pj = 0.0;
            if (cls[j] & 3u) == FULLISH { pj = p[j]; } else { pj = gas_pa_over(i32(j % P.w), i32(j / P.w)); }
            r += a * pj;
        }
    }
    if d == 0.0 {
        kind[c] = S_NONE;
        diag[c] = 0.0;
        b[c] = 0.0;
        x[c] = 0.0;
        return;
    }
    diag[c] = d;
    b[c] = r;
    x[c] = p[c];
}

/// The solution into the pressures: the cells solved take theirs, the full liquid that sleeps
/// beside them takes a hydrostatic one from them.
@compute @workgroup_size(256)
fn take_pressure(@builtin(global_invocation_id) id: vec3<u32>) {
    let c = band_cell(id.x);
    if c == 0xffffffffu { return; }
    let cl = cls[c];
    if kind[c] == S_FLUID {
        p[c] = x[c];
        return;
    }
    if cl != FULLISH { return; }
    let xx = c % P.w;
    let yy = c / P.w;
    let weight = rho(c) * G * CELL_M;
    var sum = 0.0;
    var n = 0u;
    if xx > 0u && kind[c - 1u] == S_FLUID { sum += x[c - 1u]; n += 1u; }
    if xx + 1u < P.w && kind[c + 1u] == S_FLUID { sum += x[c + 1u]; n += 1u; }
    if yy > 0u && kind[c - P.w] == S_FLUID { sum += x[c - P.w] + weight; n += 1u; }
    if yy + 1u < P.h && kind[c + P.w] == S_FLUID { sum += x[c + P.w] - weight; n += 1u; }
    if n > 0u { p[c] = sum / f32(n); }
}

fn pa_at(xi: i32, yi: i32) -> f32 {
    let j = u32(yi) * P.w + u32(xi);
    if (cls[j] & 3u) == FULLISH { return p[j]; }
    return gas_pa_over(xi, yi);
}

/// The push of the pressure on the rightward edges beside a full cell that runs.
@compute @workgroup_size(256)
fn push_u(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = P.w + 1u;
    let e = id.x + P.y0 * w;
    if e >= (P.y0 + P.rows) * w { return; }
    let xx = e % w;
    let yy = e / w;
    if xx == 0u || xx >= P.w { return; }
    let a = yy * P.w + xx - 1u;
    let bb = yy * P.w + xx;
    let ca = cls[a];
    let cb = cls[bb];
    if (ca & 3u) == SOLID || (cb & 3u) == SOLID || !(ca == (FULLISH | RUNS) || cb == (FULLISH | RUNS)) { return; }
    var r = 0.0;
    if (ca & 3u) >= WET && (cb & 3u) >= WET { r = 0.5 * (rho(a) + rho(bb)); } else if (ca & 3u) >= WET { r = rho(a); } else { r = rho(bb); }
    let s = P.dt / CELL_M;
    var val = u[e] - s / r * (pa_at(i32(xx), i32(yy)) - pa_at(i32(xx) - 1, i32(yy)));
    // (The surface the wind stirs is pushed a little at a time: it is let move.)
    let stirred = yy > 0u && ((cls[a - P.w] & 3u) == AIR || (cls[bb - P.w] & 3u) == AIR) && abs(skim_at(xx, yy)) > STIR;
    if (ca & 3u) == FULLISH && (cb & 3u) == FULLISH && abs(val) < REST && !stirred { val = 0.0; }
    u[e] = val;
}
@compute @workgroup_size(256)
fn push_v(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = P.w;
    let e = id.x + P.y0 * w;
    if e >= (P.y0 + P.rows) * w { return; }
    let xx = e % w;
    let yy = e / w;
    if yy == 0u || yy >= P.h { return; }
    let a = (yy - 1u) * P.w + xx;
    let bb = yy * P.w + xx;
    let ca = cls[a];
    let cb = cls[bb];
    if (ca & 3u) == SOLID || (cb & 3u) == SOLID || !(ca == (FULLISH | RUNS) || cb == (FULLISH | RUNS)) { return; }
    var r = 0.0;
    if (ca & 3u) >= WET && (cb & 3u) >= WET { r = 0.5 * (rho(a) + rho(bb)); } else if (ca & 3u) >= WET { r = rho(a); } else { r = rho(bb); }
    let s = P.dt / CELL_M;
    var val = v[e] - s / r * (pa_at(i32(xx), i32(yy)) - pa_at(i32(xx), i32(yy) - 1));
    if (ca & 3u) == FULLISH && (cb & 3u) == FULLISH && abs(val) < REST { val = 0.0; }
    v[e] = val;
}

// ---- The drops thrown off.

/// Air, or the thin film of spray over a surface, to a cell of liquid `m` throwing at it.
fn air_for(xi: i32, yi: i32, m: u32) -> bool {
    if xi < 0 || yi < 0 || xi >= i32(P.w) || yi >= i32(P.h) { return false; }
    let j = u32(yi) * P.w + u32(xi);
    let k = cls[j] & 3u;
    if k == AIR { return mat_of(j) == MAT_EMPTY; }
    if k == WET { return mat_of(j) == m && fill[j] < FULL / 4u; }
    return false;
}

/// What each cell throws (into `thrown`, and a record for the CPU).
@compute @workgroup_size(256)
fn throw_find(@builtin(global_invocation_id) id: vec3<u32>) {
    let c = band_cell(id.x);
    if c == 0xffffffffu { return; }
    thrown[c] = 0u;
    let cl = cls[c];
    if (cl & 3u) < WET || (cl & RUNS) == 0u { return; }
    let xx = c % P.w;
    let yy = c / P.w;
    let xi = i32(xx);
    let yi = i32(yy);
    let m = mat_of(c);
    // The speed out of the cell into the air on each side: right, left, down, up.
    var speeds = array<f32, 4>(0.0, 0.0, 0.0, 0.0);
    if air_for(xi + 1, yi, m) { speeds[0] = u[ui(xx + 1u, yy)]; }
    if air_for(xi - 1, yi, m) { speeds[1] = -u[ui(xx, yy)]; }
    if air_for(xi, yi + 1, m) { speeds[2] = v[vi(xx, yy + 1u)]; }
    if air_for(xi, yi - 1, m) { speeds[3] = -v[vi(xx, yy)]; }
    var k = 0u;
    var speed = speeds[0];
    for (var i = 1u; i < 4u; i++) {
        if speeds[i] > speed { speed = speeds[i]; k = i; }
    }
    var ox = 0.0;
    var oy = 0.0;
    if k == 0u { ox = 0.5; } else if k == 1u { ox = -0.5; } else if k == 2u { oy = 0.5; } else { oy = -0.5; }
    let f = min(fill[c], FULL);
    var vx = 0.5 * (u[ui(xx, yy)] + u[ui(xx + 1u, yy)]);
    var vy = 0.5 * (v[vi(xx, yy)] + v[vi(xx, yy + 1u)]);
    if k <= 1u { vx = speed * sign(ox); } else { vy = speed * sign(oy); }
    var share = 0.0;
    if speed > THROW { share = min((speed - THROW) / THROW, 1.0) * THROW_SHARE; }
    if k == 2u {
        let held = yy > 0u && (cls[c - P.w] & 3u) == FULLISH;
        if held { share = 0.0; } else if speed > THROW { share = 1.0; } else { share = 0.0; }
    }
    let lone = f < FULL / 4u && air_for(xi, yi + 1, m) && kind_at(xi - 1, yi) != FULLISH && kind_at(xi + 1, yi) != FULLISH && kind_at(xi, yi - 1) != FULLISH && kind_at(xi, yi + 1) != FULLISH;
    if lone && length(vec2<f32>(vx, vy)) > 0.5 { share = 1.0; }
    let q = u32(f32(f) * share);
    if q <= FULL / 50u { return; }
    let sp = length(vec2<f32>(vx, vy));
    if sp > LAUNCH { vx = vx * LAUNCH / sp; vy = vy * LAUNCH / sp; }
    let n = atomicAdd(&throws_n, 1u);
    if n >= MAX_THROWS { return; }
    thrown[c] = q;
    throws[n] = Throw(c, q, f32(xx) + 0.5 + ox, f32(yy) + 0.5 + oy, vx, vy, m, temp[c]);
}

/// The cells that threw lose what they threw.
@compute @workgroup_size(256)
fn throw_apply(@builtin(global_invocation_id) id: vec3<u32>) {
    let c = band_cell(id.x);
    if c == 0xffffffffu { return; }
    let q = thrown[c];
    if q == 0u { return; }
    note_change(c);
    let left = fill[c] - min(fill[c], q);
    let run = cls[c] & RUNS;
    if left == 0u {
        code[c] = (code[c] & ~0xffu) | MAT_EMPTY;
        fill[c] = 0u;
        cls[c] = AIR | run;
    } else {
        fill[c] = left;
        if left < FULL / HOLDS { cls[c] = WET | run; }
    }
    thrown[c] = 0u;
}

// ---- The transport: each edge passes velocity × dt × the liquid of the cell upstream.

/// What the edge between a and b passes (positive src a to b), before the limits.
fn flow(vel: f32, a: u32, bb: u32) -> i32 {
    let ca = cls[a];
    let cb = cls[bb];
    if vel == 0.0 || (ca & 3u) == SOLID || (cb & 3u) == SOLID || ((ca & RUNS) == 0u && (cb & RUNS) == 0u) { return 0; }
    var src = a;
    var to = bb;
    var sgn = 1;
    if vel < 0.0 { src = bb; to = a; sgn = -1; }
    if (cls[src] & 3u) < WET || (cls[src] & RUNS) == 0u || raw_kind(src) != K_LIQUID { return 0; }
    let kt = raw_kind(to);
    var ok = false;
    if kt == K_EMPTY { ok = true; } else if kt == K_LIQUID { ok = mat_of(to) == mat_of(src); }
    if !ok { return 0; }
    var q = fill[src];
    if (cls[to] & 3u) == FULLISH { q = FULL; } else { q = min(q, FULL); }
    let s = P.dt / CELL_M;
    return sgn * i32(min(abs(vel) * s, 1.0) * f32(q));
}

/// A cell's left and top edges.
@compute @workgroup_size(256)
fn carry_flow(@builtin(global_invocation_id) id: vec3<u32>) {
    let c = band_cell(id.x);
    if c == 0xffffffffu { return; }
    let xx = c % P.w;
    let yy = c / P.w;
    if xx == 0u {
        fu[ui(0u, yy)] = 0;
        fu[ui(P.w, yy)] = 0;
    } else {
        fu[ui(xx, yy)] = flow(u[ui(xx, yy)], c - 1u, c);
    }
    if yy == 0u {
        fv[c] = 0;
    } else {
        fv[c] = flow(v[vi(xx, yy)], c - P.w, c);
    }
}

/// An empty cell takes only one liquid: the one that gives it most.
@compute @workgroup_size(256)
fn carry_want(@builtin(global_invocation_id) id: vec3<u32>) {
    let c = band_cell(id.x);
    if c == 0xffffffffu { return; }
    want[c] = 0xffu;
    if raw_kind(c) != K_EMPTY || (cls[c] & 3u) == SOLID { return; }
    let xx = c % P.w;
    let yy = c / P.w;
    var best = 0;
    var src = 0xffffffffu;
    if xx > 0u && fu[ui(xx, yy)] > best { best = fu[ui(xx, yy)]; src = c - 1u; }
    if xx + 1u < P.w && -fu[ui(xx + 1u, yy)] > best { best = -fu[ui(xx + 1u, yy)]; src = c + 1u; }
    if yy > 0u && fv[c] > best { best = fv[c]; src = c - P.w; }
    if yy + 1u < P.h && -fv[c + P.w] > best { best = -fv[c + P.w]; src = c + P.w; }
    if src != 0xffffffffu { want[c] = mat_of(src); }
}

/// What an edge passes once the cells that take another liquid have refused theirs.
fn kept(q: i32, a: u32, bb: u32) -> i32 {
    var src = a;
    var to = bb;
    if q < 0 { src = bb; to = a; }
    if q != 0 && raw_kind(to) == K_EMPTY && want[to] != mat_of(src) { return 0; }
    return q;
}

/// No cell gives more than it holds: its outflows are cut down to that.
@compute @workgroup_size(256)
fn carry_scale(@builtin(global_invocation_id) id: vec3<u32>) {
    let c = band_cell(id.x);
    if c == 0xffffffffu { return; }
    scale[c] = 1.0;
    if (cls[c] & 3u) < WET { return; }
    let xx = c % P.w;
    let yy = c / P.w;
    var out = 0;
    if xx + 1u < P.w { out += max(kept(fu[ui(xx + 1u, yy)], c, c + 1u), 0); }
    if xx > 0u { out += max(-kept(fu[ui(xx, yy)], c - 1u, c), 0); }
    if yy + 1u < P.h { out += max(kept(fv[c + P.w], c, c + P.w), 0); }
    if yy > 0u { out += max(-kept(fv[c], c - P.w, c), 0); }
    let have = i32(fill[c]);
    if out > have { scale[c] = f32(have) / f32(out); }
}

fn settle(q: i32, a: u32, bb: u32) -> i32 {
    let k = kept(q, a, bb);
    var src = a;
    if k < 0 { src = bb; }
    return i32(f32(k) * scale[src]);
}

/// The flows kept, cut down to what their cell holds (a cell's left and top edges).
@compute @workgroup_size(256)
fn carry_settle(@builtin(global_invocation_id) id: vec3<u32>) {
    let c = band_cell(id.x);
    if c == 0xffffffffu { return; }
    let xx = c % P.w;
    let yy = c / P.w;
    if xx > 0u { fu[ui(xx, yy)] = settle(fu[ui(xx, yy)], c - 1u, c); }
    if yy > 0u { fv[c] = settle(fv[c], c - P.w, c); }
}

/// Each cell takes what flows in and gives what flows out, with the heat it carries: what it
/// becomes, kept aside until every cell has looked.
@compute @workgroup_size(256)
fn carry_gather(@builtin(global_invocation_id) id: vec3<u32>) {
    let c = band_cell(id.x);
    if c == 0xffffffffu { return; }
    let xx = c % P.w;
    let yy = c / P.w;
    var f = array<i32, 4>(0, 0, 0, 0);
    var j = array<u32, 4>(0u, 0u, 0u, 0u);
    if xx > 0u { f[0] = fu[ui(xx, yy)]; j[0] = c - 1u; }
    if xx + 1u < P.w { f[1] = -fu[ui(xx + 1u, yy)]; j[1] = c + 1u; }
    if yy > 0u { f[2] = fv[c]; j[2] = c - P.w; }
    if yy + 1u < P.h { f[3] = -fv[c + P.w]; j[3] = c + P.w; }
    if f[0] == 0 && f[1] == 0 && f[2] == 0 && f[3] == 0 {
        qnew[c] = -2147483647 - 1;
        return;
    }
    var had = 0;
    if raw_kind(c) == K_LIQUID { had = i32(fill[c]); }
    var q = had;
    var heat = f32(had) * temp[c];
    var newm = 0xffu;
    for (var k = 0; k < 4; k++) {
        if f[k] > 0 {
            q += f[k];
            heat += f32(f[k]) * temp[j[k]];
            if newm == 0xffu { newm = mat_of(j[k]); }
        } else if f[k] < 0 {
            q += f[k];
            heat += f32(f[k]) * temp[c];
        }
    }
    var t = temp[c];
    if q > 0 { t = heat / f32(q); }
    qnew[c] = q;
    tnew[c] = t;
    if had == 0 { mnew[c] = newm; } else { mnew[c] = 0xffu; }
}

/// The cells become what the gathering found.
@compute @workgroup_size(256)
fn carry_commit(@builtin(global_invocation_id) id: vec3<u32>) {
    let c = band_cell(id.x);
    if c == 0xffffffffu { return; }
    let q = qnew[c];
    if q == -2147483647 - 1 { return; }
    if q <= 0 {
        if raw_kind(c) == K_LIQUID {
            note_change(c);
            code[c] = (code[c] & ~0xffu) | MAT_EMPTY;
            fill[c] = 0u;
        }
        return;
    }
    note_change(c);
    let nm = mnew[c];
    if nm != 0xffu {
        code[c] = (code[c] & ~0xffu) | nm;
    }
    temp[c] = tnew[c];
    fill[c] = u32(q);
}

/// A cell holding more than full pushes the extra up, into air or the same liquid: one thread
/// a column, src the bottom of the band up.
@compute @workgroup_size(256)
fn overflow(@builtin(global_invocation_id) id: vec3<u32>) {
    let xx = id.x;
    if xx >= P.w { return; }
    var yy = P.y0 + P.rows;
    while yy > max(P.y0, 1u) {
        yy -= 1u;
        let c = yy * P.w + xx;
        if raw_kind(c) != K_LIQUID || fill[c] <= FULL + CRUMB { continue; }
        let j = c - P.w;
        let m = mat_of(c);
        var up = 0u;
        let mj = mat_of(j);
        if mj == MAT_EMPTY && (code[j] & MASKED) == 0u {
            up = 0u;
        } else if mj == m {
            up = fill[j];
        } else {
            continue;
        }
        let extra = fill[c] - FULL;
        if up == 0u {
            code[j] = (code[j] & ~0xffu) | m;
            temp[j] = temp[c];
        }
        fill[j] = up + extra;
        fill[c] = FULL;
        note_change(j);
        note_change(c);
    }
    // Liquid over a short cell of the same liquid sinks into it, from the top down, and a
    // crumb joins the liquid under or over it, or goes (see the CPU).
    yy = P.y0;
    while yy < P.y0 + P.rows {
        let c = yy * P.w + xx;
        yy += 1u;
        let m = mat_of(c);
        if raw_kind(c) != K_LIQUID || (code[c] & MASKED) != 0u { continue; }
        let have = fill[c];
        var to = 0xffffffffu;
        let under = c + P.w;
        let has_under = yy < P.h && mat_of(under) == m && (code[under] & MASKED) == 0u;
        if have <= CRUMB {
            if has_under {
                to = under;
            } else if c >= P.w && mat_of(c - P.w) == m && (code[c - P.w] & MASKED) == 0u {
                to = c - P.w;
            } else {
                code[c] = (code[c] & ~0xffu) | MAT_EMPTY;
                fill[c] = 0u;
                note_change(c);
                continue;
            }
        } else {
            if !has_under { continue; }
            to = under;
        }
        let there = fill[to];
        let gap = FULL - min(there, FULL);
        if (gap == 0u && have > CRUMB) || ((code[c] & AWAKE) == 0u && (code[to] & AWAKE) == 0u) { continue; }
        var moved = gap;
        if min(have, gap) + CRUMB >= have { moved = have; }
        temp[to] = (temp[to] * f32(there) + temp[c] * f32(moved)) / f32(there + moved);
        fill[to] = there + moved;
        if moved == have {
            code[c] = (code[c] & ~0xffu) | MAT_EMPTY;
            fill[c] = 0u;
        } else {
            fill[c] = have - moved;
        }
        note_change(c);
        note_change(to);
    }
}

// ---- The fastest edge of the band (for the sub-steps of the next step), a workgroup's worth
// a partial.

var<workgroup> wmax: array<f32, 256>;

@compute @workgroup_size(256)
fn fastest(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) gid: vec3<u32>) {
    let c = band_cell(id.x);
    var m = 0.0;
    if c != 0xffffffffu {
        let xx = c % P.w;
        let yy = c / P.w;
        m = max(abs(u[ui(xx, yy)]), abs(v[vi(xx, yy)]));
        if xx + 1u == P.w { m = max(m, abs(u[ui(P.w, yy)])); }
        if yy + 1u == P.h { m = max(m, abs(v[vi(xx, P.h)])); }
    }
    wmax[lid.x] = m;
    workgroupBarrier();
    for (var s = 128u; s > 0u; s >>= 1u) {
        if lid.x < s { wmax[lid.x] = max(wmax[lid.x], wmax[lid.x + s]); }
        workgroupBarrier();
    }
    if lid.x == 0u { partials[gid.x] = wmax[0]; }
}
