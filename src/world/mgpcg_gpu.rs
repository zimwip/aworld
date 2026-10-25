//! The pressure solver of the liquids on the graphics card: the multigrid-preconditioned
//! conjugate gradients of `mgpcg.rs`, each pass a compute kernel (`mgpcg.wgsl`) over a whole
//! grid of the hierarchy instead of a walk over the rows that hold liquid.
//!
//! The finest level is built on the CPU as before (`pressure` in `liquid.rs` fills in the kinds,
//! the edges, the diagonal, the right side and the start) and sent to the card whole; the coarse
//! levels are built there, the iterations run there, and the pressure comes back. A sea takes a
//! few milliseconds instead of thirty; a handful of puddles is not worth the trip, and stays on
//! the CPU (`GPU_MIN` unknowns, `AWORLD_GPU_MIN` to change it).
//!
//! The sums the conjugate gradients need (a dot product, the largest step) are added up per
//! workgroup on the card and then in order on the CPU, so a solve is the same from one run to
//! the next; it is not the CPU solver's to the bit (the card adds in another order), only to the
//! tolerance both stop at.

/// Under this many unknowns the CPU solver is faster than the trip to the card.
pub const GPU_MIN: usize = 40_000;

/// How many unknowns a system needs to go to the card.
pub fn threshold() -> usize {
    match super::gpu::wanted() {
        super::gpu::Use::Always => 0,
        _ => std::env::var("AWORLD_GPU_MIN").ok().and_then(|v| v.parse().ok()).unwrap_or(GPU_MIN),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::PressureGpu;

#[cfg(target_arch = "wasm32")]
pub struct PressureGpu;

#[cfg(target_arch = "wasm32")]
impl PressureGpu {
    pub fn new(_w: usize, _h: usize) -> Option<Self> {
        None
    }

    pub fn solve(&mut self, _fine: &mut super::mgpcg::Level, _tol: f32, _rel: f32, _max_iter: u32) -> u32 {
        0
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::super::gpu::{gpu, Gpu};
    use super::super::mgpcg::Level;
    use rayon::prelude::*;

    /// The grids of one level on the card.
    struct GpuLevel {
        w: usize,
        h: usize,
        kind: wgpu::Buffer,
        ax: wgpu::Buffer,
        ay: wgpu::Buffer,
        diag: wgpu::Buffer,
        b: wgpu::Buffer,
        x: wgpu::Buffer,
        /// The level as group 1 (or 2): its grids.
        bind: wgpu::BindGroup,
        /// The finest level with the residual as its right side and the preconditioned residual
        /// as its solution: what the V-cycle of the preconditioner works on.
        pre: Option<wgpu::BindGroup>,
    }

    /// The wiring of the kernels: see `mgpcg.wgsl` for what each does.
    struct Kernels {
        smooth: wgpu::ComputePipeline,
        restrict: wgpu::ComputePipeline,
        prolong: wgpu::ComputePipeline,
        coarsen_kind: wgpu::ComputePipeline,
        coarsen_edges: wgpu::ComputePipeline,
        coarsen_diag: wgpu::ComputePipeline,
        residual: wgpu::ComputePipeline,
        apply: wgpu::ComputePipeline,
        advance: wgpu::ComputePipeline,
        dot_rz: wgpu::ComputePipeline,
        direction: wgpu::ComputePipeline,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
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

    fn bytes<T: Copy>(v: &[T]) -> &[u8] {
        // SAFETY: the types sent are plain numbers (f32, u32, `Params`), with no padding to read.
        unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
    }

    /// Each level's parameters sit in one slot of the uniform buffer (one slot a colour), at a
    /// stride the card accepts for a dynamic offset.
    const SLOT: u64 = 256;
    const COARSEST: usize = 1024;
    const COARSEST_SWEEPS: usize = 24;
    const SMOOTH: usize = 2;
    const WG: usize = 256;

    pub struct PressureGpu {
        gpu: &'static Gpu,
        levels: Vec<GpuLevel>,
        kernels: Kernels,
        params: wgpu::Buffer,
        params_bind: wgpu::BindGroup,
        /// The conjugate gradients' vectors on the finest level (r, z, s, q) and the partial sums
        /// of the kernels that reduce, as group 3.
        cg: wgpu::BindGroup,
        /// The preconditioned residual (the V-cycle of the preconditioner starts it from zero).
        z: wgpu::Buffer,
        partials: wgpu::Buffer,
        partials_read: wgpu::Buffer,
        x_read: wgpu::Buffer,
        /// The kinds as the card reads them (a word a cell).
        kind32: Vec<u32>,
        /// How many levels this solve uses.
        used: usize,
        /// Workgroups of the reductions over the finest grid.
        groups: usize,
    }

    impl PressureGpu {
        /// The solver for a finest grid `w` x `h`, if the game has a card.
        pub fn new(w: usize, h: usize) -> Option<Self> {
            let gpu = gpu()?;
            let d = &gpu.device;
            let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("mgpcg"), source: wgpu::ShaderSource::Wgsl(include_str!("mgpcg.wgsl").into()) });
            let storage = |binding: u32| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            };
            let params_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("params"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64) },
                    count: None,
                }],
            });
            let level_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("level"), entries: &(0..6).map(storage).collect::<Vec<_>>() });
            let cg_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("cg"), entries: &(0..5).map(storage).collect::<Vec<_>>() });
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("mgpcg"), bind_group_layouts: &[&params_layout, &level_layout, &level_layout, &cg_layout], push_constant_ranges: &[] });
            let pipeline = |entry: &str| {
                d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: Some(&layout), module: &shader, entry_point: Some(entry), compilation_options: Default::default(), cache: None })
            };
            let kernels = Kernels {
                smooth: pipeline("sweep"),
                restrict: pipeline("restrict_residual"),
                prolong: pipeline("prolong"),
                coarsen_kind: pipeline("coarsen_kind"),
                coarsen_edges: pipeline("coarsen_edges"),
                coarsen_diag: pipeline("coarsen_diag"),
                residual: pipeline("residual"),
                apply: pipeline("apply"),
                advance: pipeline("advance"),
                dot_rz: pipeline("dot_rz"),
                direction: pipeline("direction"),
            };
            // The hierarchy, as `Solver::ready` makes it.
            let mut dims = vec![(w, h)];
            loop {
                let (lw, lh) = *dims.last().unwrap();
                if lw * lh <= COARSEST || lw < 4 || lh < 4 {
                    break;
                }
                dims.push((lw.div_ceil(2), lh.div_ceil(2)));
            }
            let params = d.create_buffer(&wgpu::BufferDescriptor { label: Some("params"), size: SLOT * 2 * dims.len() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let params_bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("params"),
                layout: &params_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &params, offset: 0, size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64) }) }],
            });
            let grid = |label: &str, n: usize| d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: (n * 4) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
            let n = w * h;
            let (r, z, s, q) = (grid("r", n), grid("z", n), grid("s", n), grid("q", n));
            let groups = n.div_ceil(WG);
            let partials = grid("partials", groups);
            let partials_read = d.create_buffer(&wgpu::BufferDescriptor { label: Some("partials read"), size: (groups * 4) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let x_read = d.create_buffer(&wgpu::BufferDescriptor { label: Some("x read"), size: (n * 4) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            fn entries<'a>(bufs: &[&'a wgpu::Buffer]) -> Vec<wgpu::BindGroupEntry<'a>> {
                bufs.iter().enumerate().map(|(i, b)| wgpu::BindGroupEntry { binding: i as u32, resource: b.as_entire_binding() }).collect()
            }
            let cg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("cg"), layout: &cg_layout, entries: &entries(&[&r, &z, &s, &q, &partials]) });
            let mut levels = Vec::new();
            for (l, &(lw, lh)) in dims.iter().enumerate() {
                let kind = grid("kind", lw * lh);
                let ax = grid("ax", (lw + 1) * lh);
                let ay = grid("ay", lw * (lh + 1));
                let diag = grid("diag", lw * lh);
                let b = grid("b", lw * lh);
                let x = grid("x", lw * lh);
                let bind = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("level"), layout: &level_layout, entries: &entries(&[&kind, &ax, &ay, &diag, &b, &x]) });
                let pre = (l == 0).then(|| d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("level pre"), layout: &level_layout, entries: &entries(&[&kind, &ax, &ay, &diag, &r, &z]) }));
                levels.push(GpuLevel { w: lw, h: lh, kind, ax, ay, diag, b, x, bind, pre });
            }
            Some(PressureGpu { gpu, levels, kernels, params, params_bind, cg, z, partials, partials_read, x_read, kind32: vec![0; n], used: 1, groups })
        }

        /// The slot of level `l`'s parameters for `colour`, as a dynamic offset.
        fn slot(l: usize, colour: usize) -> u32 {
            ((l * 2 + colour) as u64 * SLOT) as u32
        }

        /// Writes the parameters of every level used (and `alpha`, `beta` for the finest).
        fn write_params(&self, alpha: f32, beta: f32) {
            for l in 0..self.used {
                let lev = &self.levels[l];
                let (cw, ch) = if l + 1 < self.levels.len() { (self.levels[l + 1].w, self.levels[l + 1].h) } else { (lev.w.div_ceil(2), lev.h.div_ceil(2)) };
                for colour in 0..2 {
                    let p = Params { w: lev.w as u32, h: lev.h as u32, cw: cw as u32, ch: ch as u32, colour: colour as u32, pad: 0, alpha, beta };
                    self.gpu.queue.write_buffer(&self.params, Self::slot(l, colour) as u64, bytes(&[p]));
                }
            }
        }

        /// One kernel over level `l` (its grids as group 1, the level under as group 2), with
        /// `groups` workgroups.
        #[allow(clippy::too_many_arguments)]
        fn run(&self, pass: &mut wgpu::ComputePass, pipeline: &wgpu::ComputePipeline, l: usize, colour: usize, pre: bool, groups: (u32, u32)) {
            let lev = &self.levels[l];
            let under = &self.levels[(l + 1).min(self.levels.len() - 1)];
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &self.params_bind, &[Self::slot(l, colour)]);
            pass.set_bind_group(1, if pre { lev.pre.as_ref().unwrap() } else { &lev.bind }, &[]);
            pass.set_bind_group(2, &under.bind, &[]);
            pass.set_bind_group(3, &self.cg, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        }

        /// The workgroups of a kernel over every cell of a `w` x `h` grid, 16 x 16 a group.
        fn tiles(w: usize, h: usize) -> (u32, u32) {
            (w.div_ceil(16) as u32, h.div_ceil(16) as u32)
        }

        /// A kernel over the finest grid, a cell a thread in workgroups of `WG` (the reductions).
        fn line(&self) -> (u32, u32) {
            (self.groups as u32, 1)
        }

        /// The V-cycle on level `l` (the finest as the preconditioner's set when `pre`), as
        /// `Solver::vcycle`: sweeps, the residual down, the visit below, the correction up, sweeps.
        fn vcycle(&self, enc: &mut wgpu::CommandEncoder, l: usize, pre: bool) {
            let k = &self.kernels;
            let lev = &self.levels[l];
            let tiles = Self::tiles(lev.w, lev.h);
            if l + 1 == self.used {
                let mut pass = enc.begin_compute_pass(&Default::default());
                for _ in 0..COARSEST_SWEEPS {
                    for colour in 0..2 {
                        self.run(&mut pass, &k.smooth, l, colour, pre, tiles);
                    }
                }
                return;
            }
            let under = &self.levels[l + 1];
            let ctiles = Self::tiles(under.w, under.h);
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                for _ in 0..SMOOTH {
                    for colour in 0..2 {
                        self.run(&mut pass, &k.smooth, l, colour, pre, tiles);
                    }
                }
                self.run(&mut pass, &k.restrict, l, 0, pre, ctiles);
            }
            enc.clear_buffer(&under.x, 0, None);
            self.vcycle(enc, l + 1, false);
            let mut pass = enc.begin_compute_pass(&Default::default());
            self.run(&mut pass, &k.prolong, l, 0, pre, tiles);
            for _ in 0..SMOOTH {
                for colour in [1, 0] {
                    self.run(&mut pass, &k.smooth, l, colour, pre, tiles);
                }
            }
        }

        /// Submits the work encoded and reads the partial sums back, added up in order.
        fn finish_sum(&self, mut enc: wgpu::CommandEncoder) -> f32 {
            self.finish(&mut enc);
            self.partials_back().iter().sum()
        }

        /// The same, keeping the largest.
        fn finish_max(&self, mut enc: wgpu::CommandEncoder) -> f32 {
            self.finish(&mut enc);
            self.partials_back().iter().copied().fold(0.0, f32::max)
        }

        fn finish(&self, enc: &mut wgpu::CommandEncoder) {
            enc.copy_buffer_to_buffer(&self.partials, 0, &self.partials_read, 0, (self.groups * 4) as u64);
            let enc = std::mem::replace(enc, self.gpu.device.create_command_encoder(&Default::default()));
            self.gpu.queue.submit([enc.finish()]);
        }

        fn partials_back(&self) -> Vec<f32> {
            let slice = self.partials_read.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            self.gpu.device.poll(wgpu::Maintain::Wait);
            let out: Vec<f32> = slice.get_mapped_range().chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            self.partials_read.unmap();
            out
        }

        fn encoder(&self) -> wgpu::CommandEncoder {
            self.gpu.device.create_command_encoder(&Default::default())
        }

        /// Solves the finest level `fine` (filled in as for `Solver::solve`) on the card, from
        /// the `x` it holds, to the same stopping rules; the solution is left in `fine.x`.
        /// Returns the iterations.
        pub fn solve(&mut self, fine: &mut Level, tol: f32, rel: f32, max_iter: u32) -> u32 {
            let (w, h) = (fine.w, fine.h);
            debug_assert!(w == self.levels[0].w && h == self.levels[0].h);
            let n = w * h;
            // The finest level goes up whole; the card builds the coarse ones.
            self.kind32.par_iter_mut().zip(&fine.kind).for_each(|(d, &s)| *d = s as u32);
            let fluid = fine.kind.par_iter().filter(|&&k| k == super::super::mgpcg::FLUID).count();
            let mut nodes = fluid;
            self.used = 1;
            while nodes > COARSEST && self.used < self.levels.len() {
                nodes /= 4;
                self.used += 1;
            }
            let q = &self.gpu.queue;
            let l0 = &self.levels[0];
            q.write_buffer(&l0.kind, 0, bytes(&self.kind32));
            q.write_buffer(&l0.ax, 0, bytes(&fine.ax));
            q.write_buffer(&l0.ay, 0, bytes(&fine.ay));
            q.write_buffer(&l0.diag, 0, bytes(&fine.diag));
            q.write_buffer(&l0.b, 0, bytes(&fine.b));
            q.write_buffer(&l0.x, 0, bytes(&fine.x));
            self.write_params(0.0, 0.0);
            let k = &self.kernels;
            let mut enc = self.encoder();
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                for l in 0..self.used - 1 {
                    let under = &self.levels[l + 1];
                    let ctiles = Self::tiles(under.w, under.h);
                    self.run(&mut pass, &k.coarsen_kind, l, 0, false, ctiles);
                    self.run(&mut pass, &k.coarsen_edges, l, 0, false, Self::tiles(under.w + 1, under.h + 1));
                    self.run(&mut pass, &k.coarsen_diag, l, 0, false, ctiles);
                }
                // r = b - A x, and how far the pressure is from settled.
                self.run(&mut pass, &k.residual, 0, 0, false, self.line());
            }
            let step0 = self.finish_max(enc);
            let tol = tol.max(rel * step0);
            let mut iterations = 0;
            let mut best = step0;
            if step0 >= tol {
                // z = M⁻¹ r, s = z, rho = r · z.
                let mut rho = self.precondition();
                let mut enc = self.encoder();
                {
                    let mut pass = enc.begin_compute_pass(&Default::default());
                    self.run(&mut pass, &k.direction, 0, 0, false, self.line());
                    // q = A s, and s · q.
                    self.run(&mut pass, &k.apply, 0, 0, false, self.line());
                }
                let mut sq = self.finish_sum(enc);
                while iterations < max_iter {
                    iterations += 1;
                    if !(sq > 0.0) || !rho.is_finite() {
                        break;
                    }
                    let alpha = rho / sq;
                    // x += alpha s, r -= alpha q, and how far from settled now.
                    let step = self.advance(alpha);
                    if step < tol {
                        break;
                    }
                    // Running away (a singular system): the step is undone and the solve stops.
                    if !(step <= 8.0 * best) {
                        self.advance(-alpha);
                        break;
                    }
                    best = best.min(step);
                    let rho2 = self.precondition();
                    let beta = rho2 / rho;
                    rho = rho2;
                    // s = z + beta s, then q = A s and s · q for the next step.
                    self.write_params(0.0, beta);
                    let mut enc = self.encoder();
                    {
                        let mut pass = enc.begin_compute_pass(&Default::default());
                        self.run(&mut pass, &k.direction, 0, 0, false, self.line());
                        self.run(&mut pass, &k.apply, 0, 0, false, self.line());
                    }
                    sq = self.finish_sum(enc);
                }
            }
            // The solution back.
            let mut enc = self.encoder();
            enc.copy_buffer_to_buffer(&self.levels[0].x, 0, &self.x_read, 0, (n * 4) as u64);
            q.submit([enc.finish()]);
            let slice = self.x_read.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            self.gpu.device.poll(wgpu::Maintain::Wait);
            {
                let data = slice.get_mapped_range();
                fine.x.par_chunks_mut(w).zip(data.par_chunks_exact(w * 4)).for_each(|(row, src)| {
                    for (d, c) in row.iter_mut().zip(src.chunks_exact(4)) {
                        *d = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                    }
                });
            }
            self.x_read.unmap();
            iterations
        }

        /// x += alpha s, r -= alpha q; returns the largest Jacobi step left.
        fn advance(&self, alpha: f32) -> f32 {
            self.write_params(alpha, 0.0);
            let mut enc = self.encoder();
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                self.run(&mut pass, &self.kernels.advance, 0, 0, false, self.line());
            }
            self.finish_max(enc)
        }

        /// z = M⁻¹ r: a V-cycle from zero on the finest level with r as its right side. Returns
        /// r · z.
        fn precondition(&self) -> f32 {
            let mut enc = self.encoder();
            // (z is the fine `x` of the preconditioner's set.)
            enc.clear_buffer(&self.z, 0, None);
            self.vcycle(&mut enc, 0, true);
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                self.run(&mut pass, &self.kernels.dot_rz, 0, 0, false, self.line());
            }
            self.finish_sum(enc)
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::super::mgpcg::{Solver, FIXED, FLUID};
    use super::*;

    /// The tank of `mgpcg::tests`: water w x h under a row of air, hydrostatic once solved.
    fn tank(w: usize, h: usize) -> Solver {
        let mut s = Solver::default();
        s.ready(w, h);
        let rho = 1000.0f32;
        let f = s.fine();
        for y in 0..h {
            for x in 0..w {
                f.kind[y * w + x] = if y == 0 { FIXED } else { FLUID };
            }
            f.rows[y] = Some((0, w - 1));
        }
        for y in 0..h {
            for x in 1..w {
                f.ax[y * (w + 1) + x] = 2.0 / (2.0 * rho);
            }
        }
        for y in 1..h {
            for x in 0..w {
                f.ay[y * w + x] = if y == 1 { 1.0 / rho } else { 2.0 / (2.0 * rho) };
            }
        }
        for y in 1..h {
            for x in 0..w {
                let c = y * w + x;
                f.diag[c] = f.ax[y * (w + 1) + x] + f.ax[y * (w + 1) + x + 1] + f.ay[c] + f.ay[c + w];
                f.b[c] = if y + 1 == h { 9.81 / 6.0 } else { 0.0 };
            }
        }
        s
    }

    #[test]
    fn the_card_solves_the_wide_tank_as_the_cpu_does() {
        if super::super::gpu::gpu().is_none() {
            eprintln!("no GPU here: skipped");
            return;
        }
        for (w, h) in [(32, 16), (512, 64), (960, 200)] {
            let mut cpu = tank(w, h);
            cpu.coarsen();
            let it_cpu = cpu.solve(1.0, 0.0, 50);
            let mut sys = tank(w, h);
            let mut gpu = PressureGpu::new(w, h).unwrap();
            let it_gpu = gpu.solve(sys.fine(), 1.0, 0.0, 50);
            let (a, b) = (&cpu.levels[0].x, &sys.levels[0].x);
            let step = 1000.0 * 9.81 / 6.0;
            let mut worst = 0.0f32;
            for y in 1..h {
                for x in 0..w {
                    let c = y * w + x;
                    worst = worst.max((a[c] - b[c]).abs());
                }
                let p = b[y * w + w / 2];
                let want = step * y as f32;
                assert!((p - want).abs() < 0.02 * want + 20.0, "{w}x{h} row {y}: {p} vs {want}");
            }
            eprintln!("{w}x{h}: {it_cpu} iterations on the CPU, {it_gpu} on the card, pressures {worst:.2} Pa apart at most");
            assert!(worst < 10.0, "{w}x{h}: the card's pressure is {worst} Pa off the CPU's");
            assert!(it_gpu <= 8, "{w}x{h} took {it_gpu} iterations");
        }
    }
}
