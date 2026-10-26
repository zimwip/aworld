//! The pressure solver of the liquids on the graphics card: the multigrid-preconditioned
//! conjugate gradients of `mgpcg.rs`, each pass a compute kernel (`mgpcg.wgsl`) over a whole
//! grid of the hierarchy instead of a walk over the rows that hold liquid.
//!
//! The finest level is built on the CPU as before (`pressure` in `liquid.rs` fills in the kinds,
//! the edges, the diagonal, the right side and the start) and sent to the card whole; the coarse
//! levels are built there, the iterations run there, and the pressure comes back. A handful of
//! puddles is not worth the trip, and stays on the CPU (`GPU_MIN` unknowns, `AWORLD_GPU_MIN` to
//! change it); and the card is off unless asked for (`AWORLD_GPU=auto`, see `gpu.rs`): on an
//! integrated card (an Intel Arc B390) a solve of the big tank takes 22 ms against the CPU's 31,
//! and in the game, where a solve is a few iterations, the trip costs more than it saves. The
//! cost is not the computing (a sweep over the whole world takes 64 µs there) but the hundred
//! dependent kernel launches of a V-cycle, each ten to thirty microseconds (`bench_gpu`).
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
        coarsest: wgpu::ComputePipeline,
        restrict: wgpu::ComputePipeline,
        prolong: wgpu::ComputePipeline,
        coarsen_kind: wgpu::ComputePipeline,
        coarsen_edges: wgpu::ComputePipeline,
        coarsen_diag: wgpu::ComputePipeline,
        residual: wgpu::ComputePipeline,
        start: wgpu::ComputePipeline,
        apply: wgpu::ComputePipeline,
        alpha: wgpu::ComputePipeline,
        advance: wgpu::ComputePipeline,
        step: wgpu::ComputePipeline,
        undo: wgpu::ComputePipeline,
        dot_rz: wgpu::ComputePipeline,
        rho: wgpu::ComputePipeline,
        direction: wgpu::ComputePipeline,
    }

    /// The scalars of the conjugate gradients on the card (`sc` in the kernels).
    const SC_DONE: usize = 8;
    const SC_ITER: usize = 9;
    const SC_N: usize = 11;
    /// Iterations run on the card between two looks at whether it is done.
    const BATCH: u32 = 2;

    /// See `Params` in the kernels.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Params {
        w: u32,
        h: u32,
        cw: u32,
        ch: u32,
        colour: u32,
        y0: u32,
        rows: u32,
        groups: u32,
        cy0: u32,
        crows: u32,
        pad0: u32,
        pad1: u32,
    }

    fn bytes<T: Copy>(v: &[T]) -> &[u8] {
        // SAFETY: the types sent are plain numbers (f32, u32, `Params`), with no padding to read.
        unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
    }

    /// Each level's parameters sit in one slot of the uniform buffer (one slot a colour), at a
    /// stride the card accepts for a dynamic offset.
    const SLOT: u64 = 256;
    const COARSEST: usize = 1024;
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
        /// The scalars of the conjugate gradients, and their copy for the CPU to read.
        sc: wgpu::Buffer,
        sc_read: wgpu::Buffer,
        x_read: wgpu::Buffer,
        /// The rows sent last time (what the card holds outside them is clean): the next send
        /// covers them too, so that what was liquid and is not any more is nothing again.
        sent: Option<(usize, usize)>,
        /// The kinds as the card reads them (a word a cell).
        kind32: Vec<u32>,
        /// How many levels this solve uses.
        used: usize,
        /// Workgroups of the reductions over the finest grid, at most.
        groups: usize,
        /// The band of rows of each level this solve works on, and the workgroups of the
        /// reductions over the finest band.
        bands: Vec<(usize, usize)>,
        line_groups: usize,
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
            let cg_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("cg"), entries: &(0..6).map(storage).collect::<Vec<_>>() });
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("mgpcg"), bind_group_layouts: &[&params_layout, &level_layout, &level_layout, &cg_layout], push_constant_ranges: &[] });
            let pipeline = |entry: &str| {
                d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: Some(&layout), module: &shader, entry_point: Some(entry), compilation_options: Default::default(), cache: None })
            };
            let kernels = Kernels {
                smooth: pipeline("sweep"),
                coarsest: pipeline("sweep_coarsest"),
                restrict: pipeline("restrict_residual"),
                prolong: pipeline("prolong"),
                coarsen_kind: pipeline("coarsen_kind"),
                coarsen_edges: pipeline("coarsen_edges"),
                coarsen_diag: pipeline("coarsen_diag"),
                residual: pipeline("residual"),
                start: pipeline("start"),
                apply: pipeline("apply"),
                alpha: pipeline("alpha"),
                advance: pipeline("advance"),
                step: pipeline("step"),
                undo: pipeline("undo"),
                dot_rz: pipeline("dot_rz"),
                rho: pipeline("rho"),
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
            let sc = grid("scalars", SC_N);
            let sc_read = d.create_buffer(&wgpu::BufferDescriptor { label: Some("scalars read"), size: (SC_N * 4) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let x_read = d.create_buffer(&wgpu::BufferDescriptor { label: Some("x read"), size: (n * 4) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            fn entries<'a>(bufs: &[&'a wgpu::Buffer]) -> Vec<wgpu::BindGroupEntry<'a>> {
                bufs.iter().enumerate().map(|(i, b)| wgpu::BindGroupEntry { binding: i as u32, resource: b.as_entire_binding() }).collect()
            }
            let cg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("cg"), layout: &cg_layout, entries: &entries(&[&r, &z, &s, &q, &partials, &sc]) });
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
            Some(PressureGpu { gpu, levels, kernels, params, params_bind, cg, z, sc, sc_read, x_read, sent: None, kind32: vec![0; n], used: 1, groups, bands: Vec::new(), line_groups: 1 })
        }

        /// The slot of level `l`'s parameters for `colour`, as a dynamic offset.
        fn slot(l: usize, colour: usize) -> u32 {
            ((l * 2 + colour) as u64 * SLOT) as u32
        }

        /// The bands of every level from the finest one's (`y0..=y1`): half as high a level
        /// down, a row around; then the parameters of every level used.
        fn write_params(&mut self, y0: usize, y1: usize) {
            self.bands.clear();
            for lev in &self.levels {
                let l = self.bands.len();
                let (a, b) = (y0 >> l, y1 >> l);
                self.bands.push((a.saturating_sub(1), (b + 1).min(lev.h - 1)));
            }
            let (a, b) = self.bands[0];
            self.line_groups = ((b + 1 - a) * self.levels[0].w).div_ceil(WG);
            for l in 0..self.used {
                let lev = &self.levels[l];
                let (cw, ch) = if l + 1 < self.levels.len() { (self.levels[l + 1].w, self.levels[l + 1].h) } else { (lev.w.div_ceil(2), lev.h.div_ceil(2)) };
                let (y0, y1) = self.bands[l];
                let (cy0, cy1) = if l + 1 < self.levels.len() { self.bands[l + 1] } else { (0, 0) };
                for colour in 0..2 {
                    let p = Params { w: lev.w as u32, h: lev.h as u32, cw: cw as u32, ch: ch as u32, colour: colour as u32, y0: y0 as u32, rows: (y1 + 1 - y0) as u32, groups: self.line_groups as u32, cy0: cy0 as u32, crows: (cy1 + 1 - cy0) as u32, pad0: 0, pad1: 0 };
                    self.gpu.queue.write_buffer(&self.params, Self::slot(l, colour) as u64, bytes(&[p]));
                }
            }
        }

        /// The workgroups of a kernel over the band of level `l`, 16 x 16 cells a group.
        fn tiles_of(&self, l: usize) -> (u32, u32) {
            let (y0, y1) = self.bands[l];
            Self::tiles(self.levels[l].w, y1 + 1 - y0)
        }

        /// The same over the band of the level under `l` (a row more for the edges that close it).
        fn under_tiles(&self, l: usize, more: usize) -> (u32, u32) {
            let (y0, y1) = self.bands[l + 1];
            Self::tiles(self.levels[l + 1].w + more, y1 + 1 - y0 + more)
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

        /// A kernel over the finest band, a cell a thread in workgroups of `WG` (the reductions).
        fn line(&self) -> (u32, u32) {
            (self.line_groups as u32, 1)
        }

        /// The V-cycle on level `l` (the finest as the preconditioner's set when `pre`), as
        /// `Solver::vcycle`: sweeps, the residual down, the visit below, the correction up, sweeps.
        fn vcycle(&self, enc: &mut wgpu::CommandEncoder, l: usize, pre: bool) {
            let k = &self.kernels;
            let tiles = self.tiles_of(l);
            if l + 1 == self.used {
                // All the coarsest level's sweeps in one launch (one workgroup).
                let mut pass = enc.begin_compute_pass(&Default::default());
                self.run(&mut pass, &k.coarsest, l, 0, pre, (1, 1));
                return;
            }
            let under = &self.levels[l + 1];
            let ctiles = self.under_tiles(l, 0);
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

        /// Submits what is encoded and waits for the card to be through with it.
        #[cfg(test)]
        fn wait(&self, enc: wgpu::CommandEncoder) {
            self.gpu.queue.submit([enc.finish()]);
            self.gpu.device.poll(wgpu::Maintain::Wait);
        }

        /// What the card costs, for `bench_gpu`: a launch, a pass over the whole finest grid,
        /// a V-cycle, the sending of the whole system, the reading of the whole solution.
        /// Each is timed from the submission to the card being through, so a figure is the
        /// wall time the game would wait, not the card's own time.
        #[cfg(test)]
        pub fn bench(&mut self) {
            let (w, h) = (self.levels[0].w, self.levels[0].h);
            let cells = w * h;
            self.used = self.levels.len();
            self.kind32.iter_mut().for_each(|k| *k = super::super::mgpcg::FLUID as u32);
            self.write_params(0, h - 1);
            let k = &self.kernels;
            let q = &self.gpu.queue;
            let l0 = &self.levels[0];
            // Everything of the finest level once, so that the card has something to chew on.
            let zeros = vec![0.0f32; (w + 1) * (h + 2)];
            q.write_buffer(&l0.kind, 0, bytes(&self.kind32));
            for buf in [&l0.diag, &l0.b, &l0.x, &l0.ax, &l0.ay] {
                q.write_buffer(buf, 0, bytes(&zeros[..cells]));
            }
            q.write_buffer(&self.sc, 0, bytes(&[0.0f32; SC_N]));
            self.wait(self.encoder());
            let time = |name: &str, n: usize, unit: &str, f: &mut dyn FnMut()| {
                f();
                let t = std::time::Instant::now();
                let reps = 5;
                for _ in 0..reps {
                    f();
                }
                let each = t.elapsed().as_secs_f64() / reps as f64;
                eprintln!("{name}: {:.3} ms en tout, {:.2} µs par {unit}", each * 1e3, each * 1e6 / n as f64);
            };
            // 1. A launch: a thousand kernels of one thread, one after the other.
            let n = 1000;
            time("1000 lancements d'un noyau d'un fil", n, "lancement", &mut || {
                let mut enc = self.encoder();
                {
                    let mut pass = enc.begin_compute_pass(&Default::default());
                    for _ in 0..n {
                        self.run(&mut pass, &k.rho, 0, 0, false, (1, 1));
                    }
                }
                self.wait(enc);
            });
            // 2. A pass over the whole finest grid: a hundred sweeps (two colours each).
            let n = 100;
            let tiles = self.tiles_of(0);
            time(&format!("100 balayages de la grille {w}x{h}"), n, "balayage", &mut || {
                let mut enc = self.encoder();
                {
                    let mut pass = enc.begin_compute_pass(&Default::default());
                    for _ in 0..n {
                        for colour in 0..2 {
                            self.run(&mut pass, &k.smooth, 0, colour, false, tiles);
                        }
                    }
                }
                self.wait(enc);
            });
            // 3. A reduction over the whole finest grid, a hundred times.
            let line = self.line();
            time(&format!("100 réductions sur la grille {w}x{h}"), n, "réduction", &mut || {
                let mut enc = self.encoder();
                {
                    let mut pass = enc.begin_compute_pass(&Default::default());
                    for _ in 0..n {
                        self.run(&mut pass, &k.dot_rz, 0, 0, false, line);
                        self.run(&mut pass, &k.rho, 0, 0, false, (1, 1));
                    }
                }
                self.wait(enc);
            });
            // 4. The preconditioner: ten V-cycles over every level.
            let n = 10;
            time(&format!("10 cycles en V ({} niveaux)", self.used), n, "cycle", &mut || {
                let mut enc = self.encoder();
                for _ in 0..n {
                    self.precondition(&mut enc);
                }
                self.wait(enc);
            });
            // 5. The whole system sent: six grids of the finest level.
            let mb = (6 * cells * 4) as f64 / 1e6;
            time(&format!("envoi du système entier ({mb:.1} Mo)"), 1, "envoi", &mut || {
                let l0 = &self.levels[0];
                q.write_buffer(&l0.kind, 0, bytes(&self.kind32));
                for buf in [&l0.diag, &l0.b, &l0.x, &l0.ax, &l0.ay] {
                    q.write_buffer(buf, 0, bytes(&zeros[..cells]));
                }
                self.wait(self.encoder());
            });
            // 6. The whole solution read back.
            let mb = (cells * 4) as f64 / 1e6;
            time(&format!("lecture de la solution entière ({mb:.1} Mo)"), 1, "lecture", &mut || {
                let mut enc = self.encoder();
                enc.copy_buffer_to_buffer(&self.levels[0].x, 0, &self.x_read, 0, (cells * 4) as u64);
                self.read(enc, &self.x_read, cells);
            });
            // 7. The scalars read back (what one look at "is it done" costs).
            time("lecture des 11 scalaires", 1, "lecture", &mut || {
                let mut enc = self.encoder();
                enc.copy_buffer_to_buffer(&self.sc, 0, &self.sc_read, 0, (SC_N * 4) as u64);
                self.read(enc, &self.sc_read, SC_N);
            });
        }

        pub fn encoder(&self) -> wgpu::CommandEncoder {
            self.gpu.device.create_command_encoder(&Default::default())
        }

        /// Solves the finest level `fine` (filled in as for `Solver::solve`) on the card, from
        /// the `x` it holds, to the same stopping rules; the solution is left in `fine.x`.
        /// Returns the iterations.
        ///
        /// The card is waited for as little as can be: the scalars of the conjugate gradients
        /// stay on it, and the CPU only looks every `BATCH` iterations at whether it is done.
        pub fn solve(&mut self, fine: &mut Level, tol: f32, rel: f32, max_iter: u32) -> u32 {
            let (w, h) = (fine.w, fine.h);
            debug_assert!(w == self.levels[0].w && h == self.levels[0].h);
            // The rows to send: the ones that hold the system, and the ones sent last time.
            let Some(y0) = fine.rows.iter().position(Option::is_some) else { return 0 };
            let y1 = fine.rows.iter().rposition(Option::is_some).unwrap();
            let (sy0, sy1) = match self.sent {
                Some((a, b)) => (a.min(y0), b.max(y1)),
                None => (0, h - 1),
            };
            self.sent = Some((y0, y1));
            let band = sy0 * w..(sy1 + 1) * w;
            self.kind32[band.clone()].par_iter_mut().zip(&fine.kind[band.clone()]).for_each(|(d, &s)| *d = s as u32);
            let fluid = fine.kind[band.clone()].par_iter().filter(|&&k| k == super::super::mgpcg::FLUID).count();
            let q = &self.gpu.queue;
            let l0 = &self.levels[0];
            let at = |rows: usize| (rows * 4) as u64;
            q.write_buffer(&l0.kind, at(band.start), bytes(&self.kind32[band.clone()]));
            q.write_buffer(&l0.diag, at(band.start), bytes(&fine.diag[band.clone()]));
            q.write_buffer(&l0.b, at(band.start), bytes(&fine.b[band.clone()]));
            q.write_buffer(&l0.x, at(band.start), bytes(&fine.x[band.clone()]));
            let xband = sy0 * (w + 1)..(sy1 + 1) * (w + 1);
            q.write_buffer(&l0.ax, at(xband.start), bytes(&fine.ax[xband]));
            let yband = sy0 * w..(sy1 + 2) * w;
            q.write_buffer(&l0.ay, at(yband.start), bytes(&fine.ay[yband]));
            self.prepare(sy0, sy1, fluid, tol, rel);
            let (done, mut enc) = self.iterate(self.encoder(), max_iter);
            // The solution back (the rows of the system).
            let n = (y1 + 1 - y0) * w;
            enc.copy_buffer_to_buffer(&self.levels[0].x, at(y0 * w), &self.x_read, 0, at(n));
            let x = self.read(enc, &self.x_read, n);
            fine.x[y0 * w..(y1 + 1) * w].copy_from_slice(&x);
            done
        }

        /// The finest level's grids on the card (kind, ax, ay, diag, b, x), for a shader that
        /// builds the system there (`liquid.wgsl`).
        pub fn fine_buffers(&self) -> [&wgpu::Buffer; 6] {
            let l = &self.levels[0];
            [&l.kind, &l.ax, &l.ay, &l.diag, &l.b, &l.x]
        }

        /// Solves the system that sits in the finest level's grids on the card already (rows
        /// `y0..=y1`, about `fluid` unknowns): what `enc` holds runs first, and the encoder
        /// returned takes what comes after. Returns the iterations.
        pub fn solve_resident(&mut self, enc: wgpu::CommandEncoder, y0: usize, y1: usize, fluid: usize, tol: f32, rel: f32, max_iter: u32) -> (u32, wgpu::CommandEncoder) {
            self.sent = None;
            self.prepare(y0, y1, fluid, tol, rel);
            self.iterate(enc, max_iter)
        }

        /// The levels a system of `fluid` unknowns uses, the band of each, and the scalars.
        fn prepare(&mut self, y0: usize, y1: usize, fluid: usize, tol: f32, rel: f32) {
            let mut nodes = fluid;
            self.used = 1;
            while nodes > COARSEST && self.used < self.levels.len() {
                nodes /= 4;
                self.used += 1;
            }
            // rho, s·q, alpha, beta, step, best, tol, rel, done, iterations, undo.
            self.gpu.queue.write_buffer(&self.sc, 0, bytes(&[0.0f32, 0.0, 0.0, 0.0, 0.0, 0.0, tol, rel, 0.0, 0.0, 0.0]));
            self.write_params(y0, y1);
        }

        /// The iterations on the system in the finest level: the coarse levels built, then
        /// `BATCH` iterations at a time until done or `max_iter`. Returns the iterations and
        /// the encoder for what comes after.
        fn iterate(&self, mut enc: wgpu::CommandEncoder, max_iter: u32) -> (u32, wgpu::CommandEncoder) {
            let k = &self.kernels;
            let line = self.line();
            // The coarse levels, the first residual, the tolerance, and the first direction.
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                for l in 0..self.used - 1 {
                    let ctiles = self.under_tiles(l, 0);
                    self.run(&mut pass, &k.coarsen_kind, l, 0, false, ctiles);
                    self.run(&mut pass, &k.coarsen_edges, l, 0, false, self.under_tiles(l, 1));
                    self.run(&mut pass, &k.coarsen_diag, l, 0, false, ctiles);
                }
                self.run(&mut pass, &k.residual, 0, 0, false, line);
                self.run(&mut pass, &k.start, 0, 0, false, (1, 1));
            }
            self.precondition(&mut enc);
            let mut done = 0;
            while done < max_iter {
                // `BATCH` iterations, each: s = z + beta s; q = A s; alpha; x and r advanced; the
                // step looked at; the advance undone if it ran away; z = M⁻¹ r; rho and beta.
                // (The card counts the iterations it really did: once done, a kernel does nothing.)
                for _ in 0..BATCH.min(max_iter - done) {
                    {
                        let mut pass = enc.begin_compute_pass(&Default::default());
                        self.run(&mut pass, &k.direction, 0, 0, false, line);
                        self.run(&mut pass, &k.apply, 0, 0, false, line);
                        self.run(&mut pass, &k.alpha, 0, 0, false, (1, 1));
                        self.run(&mut pass, &k.advance, 0, 0, false, line);
                        self.run(&mut pass, &k.step, 0, 0, false, (1, 1));
                        self.run(&mut pass, &k.undo, 0, 0, false, line);
                    }
                    self.precondition(&mut enc);
                }
                enc.copy_buffer_to_buffer(&self.sc, 0, &self.sc_read, 0, (SC_N * 4) as u64);
                let sc = self.read(std::mem::replace(&mut enc, self.encoder()), &self.sc_read, SC_N);
                done = sc[SC_ITER] as u32;
                if sc[SC_DONE] != 0.0 {
                    break;
                }
            }
            (done, enc)
        }

        /// Submits what is encoded, then reads `n` numbers back from a staging buffer.
        fn read(&self, enc: wgpu::CommandEncoder, from: &wgpu::Buffer, n: usize) -> Vec<f32> {
            self.gpu.queue.submit([enc.finish()]);
            let slice = from.slice(..(n * 4) as u64);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            self.gpu.device.poll(wgpu::Maintain::Wait);
            let out: Vec<f32> = slice.get_mapped_range().chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            from.unmap();
            out
        }

        /// z = M⁻¹ r: a V-cycle from zero on the finest level with r as its right side, then
        /// rho = r · z and beta.
        fn precondition(&self, enc: &mut wgpu::CommandEncoder) {
            // (z is the fine `x` of the preconditioner's set.)
            enc.clear_buffer(&self.z, 0, None);
            self.vcycle(enc, 0, true);
            let mut pass = enc.begin_compute_pass(&Default::default());
            self.run(&mut pass, &self.kernels.dot_rz, 0, 0, false, self.line());
            self.run(&mut pass, &self.kernels.rho, 0, 0, false, (1, 1));
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

    /// The card's costs, to see what a resident solver could hope for on it:
    /// `AWORLD_GPU=1 cargo test --release --bin aworld bench_gpu -- --ignored --nocapture`.
    /// `AWORLD_BENCH_GRID=1920x1080` measures a bigger world than the game's (the tank of the
    /// last two lines is as wide, and as high in proportion).
    #[test]
    #[ignore]
    fn bench_gpu() {
        let Some(g) = super::super::gpu::gpu() else {
            eprintln!("no GPU here: skipped");
            return;
        };
        eprintln!("carte : {}", g.name);
        let (w, h) = std::env::var("AWORLD_BENCH_GRID").ok().and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))).unwrap_or((960, 540));
        eprintln!("grille : {w}x{h}");
        let mut gpu = PressureGpu::new(w, h).unwrap();
        gpu.bench();
        // And a whole solve of the big tank, as the game would ask for it.
        let (w, h) = (w, h * 200 / 540);
        let mut gpu = PressureGpu::new(w, h).unwrap();
        let mut sys = tank(w, h);
        gpu.solve(sys.fine(), 1.0, 0.0, 50);
        let t = std::time::Instant::now();
        let reps = 5;
        let mut it = 0;
        for _ in 0..reps {
            let mut sys = tank(w, h);
            it = gpu.solve(sys.fine(), 1.0, 0.0, 50);
        }
        let each = t.elapsed().as_secs_f64() / reps as f64;
        eprintln!("résolution du bassin {w}x{h} : {:.3} ms ({it} itérations, {:.3} ms chacune)", each * 1e3, each * 1e3 / it as f64);
        let mut cpu = tank(w, h);
        cpu.coarsen();
        cpu.solve(1.0, 0.0, 50);
        let t = std::time::Instant::now();
        for _ in 0..reps {
            let mut cpu = tank(w, h);
            cpu.coarsen();
            it = cpu.solve(1.0, 0.0, 50);
        }
        let each = t.elapsed().as_secs_f64() / reps as f64;
        eprintln!("le même sur le CPU : {:.3} ms ({it} itérations, {:.3} ms chacune)", each * 1e3, each * 1e3 / it as f64);
    }
}
