// The pressure solver of the liquids on the graphics card (see mgpcg_gpu.rs, and mgpcg.rs for
// the method): the same multigrid and conjugate gradients, each pass a kernel over a whole grid.
//
// Group 1 is the level a kernel works on, group 2 the level under it (half as fine), group 3 the
// vectors of the conjugate gradients on the finest level. A cell is FLUID (1) when it holds an
// unknown pressure, FIXED (2) when its pressure is known (in the right side), NONE (0) otherwise.

struct Params {
    w: u32,
    h: u32,
    cw: u32,
    ch: u32,
    colour: u32,
    pad: u32,
    alpha: f32,
    beta: f32,
}

@group(0) @binding(0) var<uniform> P: Params;

@group(1) @binding(0) var<storage, read_write> kind: array<u32>;
@group(1) @binding(1) var<storage, read_write> ax: array<f32>;
@group(1) @binding(2) var<storage, read_write> ay: array<f32>;
@group(1) @binding(3) var<storage, read_write> diag: array<f32>;
@group(1) @binding(4) var<storage, read_write> b: array<f32>;
@group(1) @binding(5) var<storage, read_write> x: array<f32>;

@group(2) @binding(0) var<storage, read_write> ckind: array<u32>;
@group(2) @binding(1) var<storage, read_write> cax: array<f32>;
@group(2) @binding(2) var<storage, read_write> cay: array<f32>;
@group(2) @binding(3) var<storage, read_write> cdiag: array<f32>;
@group(2) @binding(4) var<storage, read_write> cb: array<f32>;
@group(2) @binding(5) var<storage, read_write> cx: array<f32>;

@group(3) @binding(0) var<storage, read_write> r: array<f32>;
@group(3) @binding(1) var<storage, read_write> z: array<f32>;
@group(3) @binding(2) var<storage, read_write> s: array<f32>;
@group(3) @binding(3) var<storage, read_write> q: array<f32>;
@group(3) @binding(4) var<storage, read_write> partials: array<f32>;

const FLUID: u32 = 1u;
const WG: u32 = 256u;

var<workgroup> sh: array<f32, 256>;

// A x at the fluid cell c = (xx, y): diag x_c minus the edges to the fluid cells beside.
fn apply_x(c: u32, xx: u32, y: u32) -> f32 {
    var v = diag[c] * x[c];
    if (xx > 0u && kind[c - 1u] == FLUID) {
        v -= ax[y * (P.w + 1u) + xx] * x[c - 1u];
    }
    if (xx + 1u < P.w && kind[c + 1u] == FLUID) {
        v -= ax[y * (P.w + 1u) + xx + 1u] * x[c + 1u];
    }
    if (y > 0u && kind[c - P.w] == FLUID) {
        v -= ay[c] * x[c - P.w];
    }
    if (y + 1u < P.h && kind[c + P.w] == FLUID) {
        v -= ay[c + P.w] * x[c + P.w];
    }
    return v;
}

// The same on the search direction s.
fn apply_s(c: u32, xx: u32, y: u32) -> f32 {
    var v = diag[c] * s[c];
    if (xx > 0u && kind[c - 1u] == FLUID) {
        v -= ax[y * (P.w + 1u) + xx] * s[c - 1u];
    }
    if (xx + 1u < P.w && kind[c + 1u] == FLUID) {
        v -= ax[y * (P.w + 1u) + xx + 1u] * s[c + 1u];
    }
    if (y > 0u && kind[c - P.w] == FLUID) {
        v -= ay[c] * s[c - P.w];
    }
    if (y + 1u < P.h && kind[c + P.w] == FLUID) {
        v -= ay[c + P.w] * s[c + P.w];
    }
    return v;
}

// One Gauss-Seidel sweep over the cells of one colour: each takes the pressure that balances
// its right side against its neighbours (which the other colour holds still).
@compute @workgroup_size(16, 16)
fn sweep(@builtin(global_invocation_id) id: vec3<u32>) {
    let xx = id.x;
    let y = id.y;
    if (xx >= P.w || y >= P.h || (xx + y) % 2u != P.colour) {
        return;
    }
    let c = y * P.w + xx;
    if (kind[c] != FLUID) {
        return;
    }
    var v = b[c];
    if (xx > 0u && kind[c - 1u] == FLUID) {
        v += ax[y * (P.w + 1u) + xx] * x[c - 1u];
    }
    if (xx + 1u < P.w && kind[c + 1u] == FLUID) {
        v += ax[y * (P.w + 1u) + xx + 1u] * x[c + 1u];
    }
    if (y > 0u && kind[c - P.w] == FLUID) {
        v += ay[c] * x[c - P.w];
    }
    if (y + 1u < P.h && kind[c + P.w] == FLUID) {
        v += ay[c + P.w] * x[c + P.w];
    }
    x[c] = v / diag[c];
}

// The residual b - A x of the four fine cells under each coarse one, summed: the coarse right
// side.
@compute @workgroup_size(16, 16)
fn restrict_residual(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    let j = id.y;
    if (i >= P.cw || j >= P.ch) {
        return;
    }
    var sum = 0.0;
    for (var dy = 0u; dy < 2u; dy++) {
        for (var dx = 0u; dx < 2u; dx++) {
            let xx = 2u * i + dx;
            let y = 2u * j + dy;
            if (xx < P.w && y < P.h) {
                let c = y * P.w + xx;
                if (kind[c] == FLUID) {
                    sum += b[c] - apply_x(c, xx, y);
                }
            }
        }
    }
    cb[j * P.cw + i] = sum;
}

// The coarse correction, the same for the four cells under a coarse one, added to the fine
// solution.
@compute @workgroup_size(16, 16)
fn prolong(@builtin(global_invocation_id) id: vec3<u32>) {
    let xx = id.x;
    let y = id.y;
    if (xx >= P.w || y >= P.h) {
        return;
    }
    let c = y * P.w + xx;
    if (kind[c] == FLUID) {
        x[c] += cx[(y / 2u) * P.cw + xx / 2u];
    }
}

// The coarse grid's cells: fixed if a cell under it is fixed, else fluid if one is.
@compute @workgroup_size(16, 16)
fn coarsen_kind(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    let j = id.y;
    if (i >= P.cw || j >= P.ch) {
        return;
    }
    var k = 0u;
    for (var dy = 0u; dy < 2u; dy++) {
        for (var dx = 0u; dx < 2u; dx++) {
            let xx = 2u * i + dx;
            let y = 2u * j + dy;
            if (xx < P.w && y < P.h) {
                k = max(k, kind[y * P.w + xx]);
            }
        }
    }
    ckind[j * P.cw + i] = k;
}

fn any_fluid(a: u32, bb: u32) -> bool {
    return a != 0u && bb != 0u && (a == FLUID || bb == FLUID);
}

// The coarse edges: the mean of the two fine edges that cross each face (nothing at the border
// of the grid, nothing between two cells that are not an unknown and its neighbour). The
// thread (i, j) does the left edge of coarse cell (i, j) and the edge over it; the last column
// and row of threads do the edges that close the grid.
@compute @workgroup_size(16, 16)
fn coarsen_edges(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    let j = id.y;
    if (i > P.cw || j > P.ch) {
        return;
    }
    if (i <= P.cw && j < P.ch) {
        var v = 0.0;
        if (i >= 1u && i + 1u <= P.cw && any_fluid(ckind[j * P.cw + i - 1u], ckind[j * P.cw + i])) {
            for (var dy = 0u; dy < 2u; dy++) {
                let y = 2u * j + dy;
                if (y < P.h && 2u * i < P.w) {
                    v += ax[y * (P.w + 1u) + 2u * i];
                }
            }
            v *= 0.5;
        }
        cax[j * (P.cw + 1u) + i] = v;
    }
    if (i < P.cw && j <= P.ch) {
        var v = 0.0;
        if (j >= 1u && j + 1u <= P.ch && any_fluid(ckind[(j - 1u) * P.cw + i], ckind[j * P.cw + i])) {
            for (var dx = 0u; dx < 2u; dx++) {
                let xx = 2u * i + dx;
                if (xx < P.w && 2u * j < P.h + 1u) {
                    v += ay[2u * j * P.w + xx];
                }
            }
            v *= 0.5;
        }
        cay[j * P.cw + i] = v;
    }
}

// The coarse diagonal: the sum of the edges of each fluid cell (a cell without an edge is
// nothing).
@compute @workgroup_size(16, 16)
fn coarsen_diag(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    let j = id.y;
    if (i >= P.cw || j >= P.ch) {
        return;
    }
    let c = j * P.cw + i;
    if (ckind[c] != FLUID) {
        cdiag[c] = 0.0;
        return;
    }
    let d = cax[j * (P.cw + 1u) + i] + cax[j * (P.cw + 1u) + i + 1u] + cay[c] + cay[c + P.cw];
    cdiag[c] = d;
    if (d <= 0.0) {
        ckind[c] = 0u;
    }
}

// The sum (or the largest) of what each thread of a workgroup holds, into one partial per
// workgroup; the CPU adds the partials up in order.
fn reduce_sum(local: u32, group: u32, v: f32) {
    sh[local] = v;
    workgroupBarrier();
    for (var stride = WG / 2u; stride > 0u; stride /= 2u) {
        if (local < stride) {
            sh[local] += sh[local + stride];
        }
        workgroupBarrier();
    }
    if (local == 0u) {
        partials[group] = sh[0];
    }
}

fn reduce_max(local: u32, group: u32, v: f32) {
    sh[local] = v;
    workgroupBarrier();
    for (var stride = WG / 2u; stride > 0u; stride /= 2u) {
        if (local < stride) {
            sh[local] = max(sh[local], sh[local + stride]);
        }
        workgroupBarrier();
    }
    if (local == 0u) {
        partials[group] = sh[0];
    }
}

// r = b - A x, and how far each cell is from settled (its Jacobi step), the largest kept.
@compute @workgroup_size(256)
fn residual(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>) {
    let c = id.x;
    var m = 0.0;
    if (c < P.w * P.h) {
        if (kind[c] == FLUID) {
            let v = b[c] - apply_x(c, c % P.w, c / P.w);
            r[c] = v;
            m = abs(v / diag[c]);
        } else {
            r[c] = 0.0;
        }
    }
    reduce_max(lid.x, wid.x, m);
}

// q = A s, and s · q.
@compute @workgroup_size(256)
fn apply(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>) {
    let c = id.x;
    var d = 0.0;
    if (c < P.w * P.h) {
        if (kind[c] == FLUID) {
            let v = apply_s(c, c % P.w, c / P.w);
            q[c] = v;
            d = s[c] * v;
        } else {
            q[c] = 0.0;
        }
    }
    reduce_sum(lid.x, wid.x, d);
}

// x += alpha s, r -= alpha q, and how far from settled now.
@compute @workgroup_size(256)
fn advance(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>) {
    let c = id.x;
    var m = 0.0;
    if (c < P.w * P.h && kind[c] == FLUID) {
        x[c] += P.alpha * s[c];
        let v = r[c] - P.alpha * q[c];
        r[c] = v;
        m = abs(v / diag[c]);
    }
    reduce_max(lid.x, wid.x, m);
}

// r · z (z being the fine `x` of the preconditioner's level set, bound here as z).
@compute @workgroup_size(256)
fn dot_rz(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>) {
    let c = id.x;
    var d = 0.0;
    if (c < P.w * P.h && kind[c] == FLUID) {
        d = r[c] * z[c];
    }
    reduce_sum(lid.x, wid.x, d);
}

// s = z + beta s.
@compute @workgroup_size(256)
fn direction(@builtin(global_invocation_id) id: vec3<u32>) {
    let c = id.x;
    if (c < P.w * P.h) {
        if (kind[c] == FLUID) {
            s[c] = z[c] + P.beta * s[c];
        } else {
            s[c] = 0.0;
        }
    }
}
