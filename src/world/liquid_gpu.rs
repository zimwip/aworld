//! The liquids' turn on the graphics card, whole (kernels in `liquid.wgsl`, the pressure's
//! solver in `mgpcg_gpu.rs`): the grids live on the card from one step to the next, the CPU
//! sends it the state of the cells at the start of a step (what the rest of the game changed) and
//! reads back at the end what the liquids changed, the drops thrown off and the pressure.
//!
//! A step works on a band of rows: the rows that hold liquid that runs, a margin around them
//! (the liquid crosses at most a cell a sub-step), and the band of the step before (so that
//! what moved out of it is cleaned up). Everything sent and read back is that band.
//!
//! The web build has no card yet (see `gpu.rs`).

#[cfg(not(target_arch = "wasm32"))]
pub use native::LiquidGpu;

#[cfg(target_arch = "wasm32")]
pub struct LiquidGpu {
    pub patches_up: Vec<Patch>,
}

#[cfg(target_arch = "wasm32")]
impl LiquidGpu {
    pub fn new() -> Option<Self> {
        None
    }

    pub fn rows(&self) -> Option<(usize, usize)> {
        None
    }

    pub fn take_over(&mut self, _liq: &super::liquid::Liquids) {}

    pub fn hand_back(&mut self, _liq: &mut super::liquid::Liquids) {}
}

#[cfg(target_arch = "wasm32")]
impl super::World {
    pub(super) fn step_liquids_on_card(&mut self, _liq: &mut super::liquid::Liquids, _card: &mut LiquidGpu, _step: f32) {}
    pub(super) fn collect_liquids(&mut self, _liq: &mut super::liquid::Liquids, _card: &mut LiquidGpu) {}
}

/// A change to an edge's velocity the CPU asks for at the start of a step (a drop that landed,
/// a cell that fell): see `Patch` in the kernels.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Patch {
    pub edge: u32,
    pub mode: u32,
    pub value: f32,
    pub weight: f32,
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::super::gpu::{gpu, Gpu};
    use super::super::liquid::*;
    use super::super::mgpcg_gpu::{PressureGpu, Stamper, SC_DONE, SC_ITER, SC_N};
    use super::super::*;
    use super::Patch;
    use rayon::prelude::*;

    const WG: usize = 256;
    const SLOT: u64 = 256;
    const MAX_THROWS: usize = 16384;
    const MAX_PATCHES: usize = 1 << 16;
    const MASKED: u32 = 0x100;
    const OPEN: u32 = 0x200;
    const AWAKE: u32 = 0x400;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Params {
        w: u32,
        h: u32,
        y0: u32,
        rows: u32,
        dt: f32,
        colour: u32,
        patches: u32,
        groups: u32,
        stamp: u32,
        sent: u32,
        time: f32,
        pad: u32,
    }

    /// A cell as it goes to the card and comes back (see `CellRecord` in the kernels).
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CellRecord {
        cell: u32,
        code: u32,
        fill: u32,
        temp: f32,
        gaspa: f32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
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

    fn bytes<T: Copy>(v: &[T]) -> &[u8] {
        // SAFETY: the types sent are plain numbers and `repr(C)` structs of them, with no padding.
        unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
    }

    /// A step the card works on: the band, and where each part of its result sits in `read`
    /// (in words).
    struct Pending {
        y0: usize,
        rows: usize,
        groups: usize,
        throws_at: usize,
        /// The band of `code`, `fill`, `temp` and `mark` as the card left them.
        cells_at: usize,
        partials_at: usize,
        /// The stamp of the step: the cells whose mark carries it changed.
        stamp: u32,
        sc_at: usize,
        /// The card's time stamps (in words), and what each marks the end of.
        stamps_at: usize,
        marks: Vec<&'static str>,
        total: u64,
    }

    struct Kernels {
        apply_cells: wgpu::ComputePipeline,
        patch: wgpu::ComputePipeline,
        classify: wgpu::ComputePipeline,
        forward_u: wgpu::ComputePipeline,
        forward_v: wgpu::ComputePipeline,
        back_u: wgpu::ComputePipeline,
        back_v: wgpu::ComputePipeline,
        thickness: wgpu::ComputePipeline,
        viscous_u: wgpu::ComputePipeline,
        viscous_v: wgpu::ComputePipeline,
        system_cells: wgpu::ComputePipeline,
        system_edges: wgpu::ComputePipeline,
        assemble: wgpu::ComputePipeline,
        take_pressure: wgpu::ComputePipeline,
        push_u: wgpu::ComputePipeline,
        push_v: wgpu::ComputePipeline,
        throw_find: wgpu::ComputePipeline,
        throw_apply: wgpu::ComputePipeline,
        carry_flow: wgpu::ComputePipeline,
        carry_want: wgpu::ComputePipeline,
        carry_scale: wgpu::ComputePipeline,
        carry_settle: wgpu::ComputePipeline,
        carry_gather: wgpu::ComputePipeline,
        carry_commit: wgpu::ComputePipeline,
        overflow: wgpu::ComputePipeline,
        fastest: wgpu::ComputePipeline,
    }

    pub struct LiquidGpu {
        gpu: &'static Gpu,
        pressure: PressureGpu,
        kernels: Kernels,
        params: wgpu::Buffer,
        binds: [wgpu::BindGroup; 4],
        // The cells (group 1).
        code: wgpu::Buffer,
        fill: wgpu::Buffer,
        temp: wgpu::Buffer,
        /// Only the kernels read it, through `binds`.
        _gaspa: wgpu::Buffer,
        tables: wgpu::Buffer,
        class: wgpu::Buffer,
        p: wgpu::Buffer,
        // The velocities (group 3).
        u: wgpu::Buffer,
        v: wgpu::Buffer,
        u2: wgpu::Buffer,
        v2: wgpu::Buffer,
        u3: wgpu::Buffer,
        v3: wgpu::Buffer,
        patches: wgpu::Buffer,
        throws_n: wgpu::Buffer,
        throws: wgpu::Buffer,
        partials: wgpu::Buffer,
        sent_cells: wgpu::Buffer,
        /// What comes back: the band's class and p, the throws, the changed cells, the partials.
        mark: wgpu::Buffer,
        read: wgpu::Buffer,
        /// The time stamps between the kernels, with `AWORLD_DEBUG_CARD` where the card offers
        /// them: the set, and the buffer they resolve into.
        stamps: Option<(wgpu::QuerySet, wgpu::Buffer)>,
        /// The band of rows of the last step.
        sent: Option<(usize, usize)>,
        /// The rows whose cells the card holds (every band so far).
        held: Option<(usize, usize)>,
        /// The step's stamp (the kernels report a cell changed once a step).
        stamp: u32,
        /// The cells as the card holds them, and the cells as the game has them now.
        code_up: Vec<u32>,
        fill_up: Vec<u32>,
        temp_up: Vec<f32>,
        gaspa_up: Vec<f32>,
        code_new: Vec<u32>,
        fill_new: Vec<u32>,
        temp_new: Vec<f32>,
        gaspa_new: Vec<f32>,
        /// The fastest edge at the end of the last step (for the sub-steps of the next).
        pub fastest: f32,
        /// The step the card is working on, if any: where its results land in `read`.
        pending: Option<Pending>,
        /// The pressure's iterations to encode (the card is not looked at mid-step): what the
        /// last step took, one more if it was not done.
        iters: u32,
        /// The patches to apply at the start of the next step.
        pub patches_up: Vec<Patch>,
    }

    impl LiquidGpu {
        /// The liquids' turn on the card, if the game has one.
        pub fn new() -> Option<Self> {
            let gpu = gpu()?;
            let pressure = PressureGpu::new(W(), H())?;
            let d = &gpu.device;
            assert!(Mat::Water as u8 == 2 && Mat::Stone as u8 == 7 && Mat::Empty as u8 == 0, "liquid.wgsl names these materials by their number");
            // A shader that does not compile would leave a card that does nothing, quietly: the
            // liquids then stay on the CPU, and the error is said once.
            d.push_error_scope(wgpu::ErrorFilter::Validation);
            let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("liquid"), source: wgpu::ShaderSource::Wgsl(crate::world::units::wgsl(include_str!("liquid.wgsl")).into()) });
            if let Some(e) = pollster::block_on(d.pop_error_scope()) {
                eprintln!("aworld: the liquids' shader does not compile, the liquids stay on the CPU: {e}");
                return None;
            }
            let storage = |binding: u32| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            };
            let uniform = wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64) },
                count: None,
            };
            let mut g0 = vec![uniform];
            g0.extend((1..7).map(storage));
            let layouts = [
                d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("liquid solver"), entries: &g0 }),
                d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("liquid cells"), entries: &(0..9).map(storage).collect::<Vec<_>>() }),
                d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("liquid scratch"), entries: &(0..10).map(storage).collect::<Vec<_>>() }),
                d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("liquid edges"), entries: &(0..10).map(storage).collect::<Vec<_>>() }),
            ];
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("liquid"), bind_group_layouts: &[&layouts[0], &layouts[1], &layouts[2], &layouts[3]], push_constant_ranges: &[] });
            let pipeline = |entry: &str| {
                d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: Some(&layout), module: &shader, entry_point: Some(entry), compilation_options: Default::default(), cache: None })
            };
            let kernels = Kernels {
                apply_cells: pipeline("apply_cells"),
                patch: pipeline("apply_patches"),
                classify: pipeline("classify"),
                forward_u: pipeline("forward_u"),
                forward_v: pipeline("forward_v"),
                back_u: pipeline("back_u"),
                back_v: pipeline("back_v"),
                thickness: pipeline("thickness"),
                viscous_u: pipeline("viscous_u"),
                viscous_v: pipeline("viscous_v"),
                system_cells: pipeline("system_cells"),
                system_edges: pipeline("system_edges"),
                assemble: pipeline("assemble"),
                take_pressure: pipeline("take_pressure"),
                push_u: pipeline("push_u"),
                push_v: pipeline("push_v"),
                throw_find: pipeline("throw_find"),
                throw_apply: pipeline("throw_apply"),
                carry_flow: pipeline("carry_flow"),
                carry_want: pipeline("carry_want"),
                carry_scale: pipeline("carry_scale"),
                carry_settle: pipeline("carry_settle"),
                carry_gather: pipeline("carry_gather"),
                carry_commit: pipeline("carry_commit"),
                overflow: pipeline("overflow"),
                fastest: pipeline("fastest"),
            };
            let grid = |label: &str, n: usize| d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: (n * 4) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
            let n = W() * H();
            let (nu_, nv) = ((W() + 1) * H(), W() * (H() + 1));
            let params = d.create_buffer(&wgpu::BufferDescriptor { label: Some("liquid params"), size: SLOT * 2, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let (code, fill, temp, gaspa, class, p, nu, rhs) = (grid("code", n), grid("fill", n), grid("temp", n), grid("gaspa", n), grid("class", n), grid("p", n), grid("nu", n), grid("rhs", n));
            // The materials (256 words), then the wind over the liquids and the first hard row of
            // each column (flow.rs: `skim`, `sky`).
            let tables = grid("tables", 256 + flow::FW() * flow::FH() + W());
            let (want, scale, qnew, tnew, fu, fv, thrown, mnew) = (grid("want", n), grid("scale", n), grid("qnew", n), grid("tnew", n), grid("fu", nu_), grid("fv", nv), grid("thrown", n), grid("mnew", n));
            let groups = n.div_ceil(WG);
            let partials = grid("partials", groups);
            let mark = grid("mark", n);
            let (u, v, u2, v2, u3, v3) = (grid("u", nu_), grid("v", nv), grid("u2", nu_), grid("v2", nv), grid("u3", nu_), grid("v3", nv));
            let patches = grid("patches", MAX_PATCHES * 4);
            let throws_n = grid("throws n", 1);
            let throws = grid("throws", MAX_THROWS * 8);
            let sent_cells = grid("sent cells", n * 5);
            let read_size = (6 * n + 1 + MAX_THROWS * 8 + groups + SC_N) * 4 + MAX_STAMPS * 8;
            let stamps = gpu.timestamps.then(|| {
                let set = d.create_query_set(&wgpu::QuerySetDescriptor { label: Some("liquid stamps"), ty: wgpu::QueryType::Timestamp, count: MAX_STAMPS as u32 });
                let buf = d.create_buffer(&wgpu::BufferDescriptor { label: Some("liquid stamps"), size: (MAX_STAMPS * 8) as u64, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
                (set, buf)
            });
            let read = d.create_buffer(&wgpu::BufferDescriptor { label: Some("liquid read"), size: read_size as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            // The materials, for the kernels.
            let mut table = vec![0u32; 256];
            for &m in &Mat::ALL {
                let i = m as usize;
                table[i] = match m.kind() {
                    Kind::Empty => 0,
                    Kind::Static => 1,
                    Kind::Powder => 2,
                    Kind::Liquid => 3,
                    Kind::Gas => 4,
                    Kind::Flame => 5,
                };
                table[64 + i] = density(m).to_bits();
                table[128 + i] = props(m, 15.0).1.to_bits();
                table[192 + i] = (m == Mat::Lava) as u32;
            }
            gpu.queue.write_buffer(&tables, 0, bytes(&table));
            fn entries<'a>(bufs: &[&'a wgpu::Buffer]) -> Vec<wgpu::BindGroupEntry<'a>> {
                bufs.iter().enumerate().map(|(i, b)| wgpu::BindGroupEntry { binding: i as u32, resource: b.as_entire_binding() }).collect()
            }
            let fine = pressure.fine_buffers();
            let mut e0 = vec![wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &params, offset: 0, size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64) }) }];
            e0.extend(fine.iter().enumerate().map(|(i, b)| wgpu::BindGroupEntry { binding: i as u32 + 1, resource: b.as_entire_binding() }));
            let binds = [
                d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("liquid solver"), layout: &layouts[0], entries: &e0 }),
                d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("liquid cells"), layout: &layouts[1], entries: &entries(&[&code, &fill, &temp, &gaspa, &class, &p, &nu, &rhs, &tables]) }),
                d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("liquid scratch"), layout: &layouts[2], entries: &entries(&[&want, &scale, &qnew, &tnew, &fu, &fv, &thrown, &mnew, &partials, &mark]) }),
                d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("liquid edges"), layout: &layouts[3], entries: &entries(&[&u, &v, &u2, &v2, &u3, &v3, &patches, &throws_n, &throws, &sent_cells]) }),
            ];
            Some(LiquidGpu {
                gpu,
                pressure,
                kernels,
                params,
                binds,
                code,
                fill,
                temp,
                _gaspa: gaspa,
                tables,
                class,
                p,
                u,
                v,
                u2,
                v2,
                u3,
                v3,
                patches,
                throws_n,
                throws,
                partials,
                sent_cells,
                mark,
                read,
                stamps,
                sent: None,
                held: None,
                stamp: 0,
                code_up: vec![0; n],
                fill_up: vec![0; n],
                temp_up: vec![0.0; n],
                gaspa_up: vec![0.0; n],
                code_new: vec![0; n],
                fill_new: vec![0; n],
                temp_new: vec![0.0; n],
                gaspa_new: vec![0.0; n],
                fastest: 0.0,
                pending: None,
                iters: 4,
                patches_up: Vec::new(),
            })
        }

        /// The liquids come over from the CPU: their velocities go to the card, and every cell
        /// of the next band goes up again (what the card held is from before it gave them back).
        pub fn take_over(&mut self, liq: &Liquids) {
            if liq.u.len() == (W() + 1) * H() {
                self.gpu.queue.write_buffer(&self.u, 0, bytes(&liq.u));
                self.gpu.queue.write_buffer(&self.v, 0, bytes(&liq.v));
                self.gpu.queue.write_buffer(&self.p, 0, bytes(&liq.p));
            }
            self.fastest = liq.u.iter().chain(&liq.v).fold(0.0f32, |a, v| a.max(v.abs()));
            (self.sent, self.held, self.pending) = (None, None, None);
            self.patches_up.clear();
        }

        /// The liquids go back to the CPU (the card's last step was taken in): their velocities
        /// come back from the card.
        pub fn hand_back(&mut self, liq: &mut Liquids) {
            let d = &self.gpu.device;
            let (nu_, nv) = ((W() + 1) * H(), W() * (H() + 1));
            let back = d.create_buffer(&wgpu::BufferDescriptor { label: Some("liquid velocities back"), size: ((nu_ + nv) * 4) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let mut enc = self.encoder();
            enc.copy_buffer_to_buffer(&self.u, 0, &back, 0, (nu_ * 4) as u64);
            enc.copy_buffer_to_buffer(&self.v, 0, &back, (nu_ * 4) as u64, (nv * 4) as u64);
            self.gpu.queue.submit([enc.finish()]);
            let slice = back.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            d.poll(wgpu::Maintain::Wait);
            {
                let data = slice.get_mapped_range();
                let word = |k: usize| f32::from_le_bytes([data[k * 4], data[k * 4 + 1], data[k * 4 + 2], data[k * 4 + 3]]);
                liq.u = (0..nu_).map(word).collect();
                liq.v = (nu_..nu_ + nv).map(word).collect();
            }
            back.unmap();
            (self.sent, self.held, self.pending) = (None, None, None);
            // (The classes of the zones at rest were the card's: all are worked out again.)
            liq.classed.clear();
        }

        /// The rows of the last step given to the card.
        pub fn rows(&self) -> Option<(usize, usize)> {
            self.sent
        }

        fn encoder(&self) -> wgpu::CommandEncoder {
            self.gpu.device.create_command_encoder(&Default::default())
        }

        /// One kernel over `n` threads, with the parameters of `colour`. The bind groups are
        /// set once a pass (and the parameters' slot once a colour): every kernel shares the
        /// layout, and setting them again at each launch was most of the encoding's cost.
        fn run(&self, pass: &mut Pass, pipeline: &wgpu::ComputePipeline, colour: u32, n: usize) {
            pass.pass.set_pipeline(pipeline);
            if pass.colour != Some(colour) {
                pass.pass.set_bind_group(0, &self.binds[0], &[colour * SLOT as u32]);
                pass.colour = Some(colour);
            }
            if !pass.bound {
                pass.pass.set_bind_group(1, &self.binds[1], &[]);
                pass.pass.set_bind_group(2, &self.binds[2], &[]);
                pass.pass.set_bind_group(3, &self.binds[3], &[]);
                pass.bound = true;
            }
            pass.pass.dispatch_workgroups(n.div_ceil(WG) as u32, 1, 1);
        }
    }

    /// A compute pass of the liquids' step, remembering what is bound.
    struct Pass<'a> {
        pass: wgpu::ComputePass<'a>,
        colour: Option<u32>,
        bound: bool,
    }

    impl<'a> Pass<'a> {
        fn new(enc: &'a mut wgpu::CommandEncoder) -> Self {
            Pass { pass: enc.begin_compute_pass(&Default::default()), colour: None, bound: false }
        }
    }

    /// A cell goes up to the card again when its heat (°C) or the pressure of its pocket (Pa)
    /// moved by more than this since it was sent.
    const TEMP_GAP: f32 = 0.5;
    const GASPA_GAP: f32 = 20.0;

    /// The most time stamps a step writes (one before the first kernel, one after each phase of
    /// each sub-step, one after the copies back).
    const MAX_STAMPS: usize = 2 + 5 * MAX_SUB_ALL + 2 + 2 + MAX_TURNS * 4 * (MAX_ITER as usize + 1);
    /// The card's time in each phase, in ns, summed over the steps (`AWORLD_DEBUG_CARD`).
    static CARD: [std::sync::atomic::AtomicU64; 10] = [const { std::sync::atomic::AtomicU64::new(0) }; 10];
    const PHASES: [&str; 10] = ["envoi", "forces", "viscosité", "pression", "pression: préparation", "pression: gradients", "pression: lissage fin", "pression: niveaux grossiers", "transport", "retour"];
    /// The audit of the water at the collect (`AWORLD_DEBUG_MASS`).
    static AUDIT: [std::sync::atomic::AtomicI64; 5] = [const { std::sync::atomic::AtomicI64::new(0) }; 5];
    static DBG_ITER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static DBG: [std::sync::atomic::AtomicU64; 8] = [const { std::sync::atomic::AtomicU64::new(0) }; 8];
    fn tick(k: usize, t: std::time::Instant) -> std::time::Instant {
        DBG[k].fetch_add(t.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
        std::time::Instant::now()
    }

    impl World {
        /// The liquids' turn on the card: the result of the step the card worked on comes in,
        /// the drops fly, and the card is given this step (the CPU goes on without waiting).
        pub(in super::super) fn step_liquids_on_card(&mut self, liq: &mut Liquids, card: &mut LiquidGpu, step: f32) {
            self.collect_liquids(liq, card);
            self.fly_drops(liq, step);
            self.submit_liquids(liq, card, step);
        }

        /// What the card did with the step it was given: the cells it changed, the drops it
        /// threw, the classes and the pressure, into the world.
        pub(in super::super) fn collect_liquids(&mut self, liq: &mut Liquids, card: &mut LiquidGpu) {
            let Some(pend) = card.pending.take() else { return };
            let audit = crate::config::config().debug_mass;
            let units = |w: &World| (0..W() * H()).filter(|&i| w.cells[i].m == Mat::Water).map(|i| w.liquid_fill(i) as u64).sum::<u64>();
            let before = if audit { units(self) + liq.drops.iter().filter(|d| d.m == Mat::Water).map(|d| d.q as u64).sum::<u64>() } else { 0 };
            let clock = self.prof.start();
            let slice = card.read.slice(..pend.total);
            let mut t = std::time::Instant::now();
            card.gpu.device.poll(wgpu::Maintain::Wait);
            t = tick(5, t);
            self.prof.stop(prof::LIQ_CARD, clock);
            let clock = self.prof.start();
            let (y0, rows, groups) = (pend.y0, pend.rows, pend.groups);
            let n = rows * W();
            let cells = y0 * W()..(y0 + rows) * W();
            let data = slice.get_mapped_range();
            let data_ref: &[u8] = &data;
            let word = |k: usize| u32::from_le_bytes([data_ref[k * 4], data_ref[k * 4 + 1], data_ref[k * 4 + 2], data_ref[k * 4 + 3]]);
            let count = (word(pend.throws_at - 1) as usize).min(MAX_THROWS);
            let mut throws: Vec<Throw> = (0..count)
                .map(|t| {
                    let k = pend.throws_at + t * 8;
                    Throw { cell: word(k), q: word(k + 1), x: f32::from_bits(word(k + 2)), y: f32::from_bits(word(k + 3)), vx: f32::from_bits(word(k + 4)), vy: f32::from_bits(word(k + 5)), mat: word(k + 6), temp: f32::from_bits(word(k + 7)) }
                })
                .collect();
            card.fastest = (0..groups).map(|g| f32::from_bits(word(pend.partials_at + g))).fold(0.0f32, f32::max);
            let iterations = f32::from_bits(word(pend.sc_at + SC_ITER)) as u32;
            DBG_ITER.fetch_add(iterations as u64, std::sync::atomic::Ordering::Relaxed);
            if card.stamps.is_some() && pend.marks.len() > 1 {
                // The card's time in each phase: the stamps' differences, in ns.
                let period = card.gpu.queue.get_timestamp_period() as f64;
                let at = |k: usize| (word(pend.stamps_at + 2 * k) as u64 | (word(pend.stamps_at + 2 * k + 1) as u64) << 32) as f64 * period;
                let mut last = at(0);
                for (k, what) in pend.marks.iter().enumerate().skip(1) {
                    let now = at(k);
                    if let Some(phase) = PHASES.iter().position(|p| p == what) {
                        CARD[phase].fetch_add((now - last).max(0.0) as u64, std::sync::atomic::Ordering::Relaxed);
                    }
                    last = now;
                }
            }
            let done = f32::from_bits(word(pend.sc_at + SC_DONE)) != 0.0;
            card.iters = if done { iterations.max(1) } else { (card.iters + 1).min(MAX_ITER) };
            self.prof.stop_n(prof::LIQ_ITERATIONS, None, iterations);
            liq.class[cells.clone()].par_iter_mut().enumerate().for_each(|(j, c)| *c = word(j) as u8);
            liq.p[cells.clone()].par_iter_mut().enumerate().for_each(|(j, c)| *c = f32::from_bits(word(n + j)));
            // What the card changed (the cells whose mark carries the step's stamp), into the
            // cells and into what the card is known to hold; row by row in parallel, the zones
            // woken and the states cleared after. A cell the rest of the game changed meanwhile
            // keeps the game's version: the card will be told at the next step. What the card
            // moved (fill, heat) is added to what the game has now, not put in its place: the
            // rest of the game went on meanwhile (the heat spread, the lava cooled, the sun
            // dried the surface), and that would be lost at every step.
            let frame = self.frame;
            let (cells_at, stamp) = (pend.cells_at, pend.stamp);
            let (world_cells, world_fill, world_temp) = (&mut self.cells[cells.clone()], &mut self.fill[cells.clone()], &mut self.temp[cells.clone()]);
            let (code_up, fill_up, temp_up) = (&mut card.code_up[cells.clone()], &mut card.fill_up[cells.clone()], &mut card.temp_up[cells.clone()]);
            let (changed, woken, cleared): (usize, Vec<usize>, Vec<(usize, u32)>) = world_cells
                .par_chunks_mut(W())
                .zip(world_fill.par_chunks_mut(W()))
                .zip(world_temp.par_chunks_mut(W()))
                .zip(code_up.par_chunks_mut(W()))
                .zip(fill_up.par_chunks_mut(W()))
                .zip(temp_up.par_chunks_mut(W()))
                .enumerate()
                .map(|(yy, (((((wc, wf), wt), cu), fu), tu))| {
                    let (mut changed, mut woken, mut cleared) = (0, Vec::new(), Vec::new());
                    let mut last_zone = usize::MAX;
                    for x in 0..W() {
                        let j = yy * W() + x;
                        if word(cells_at + 3 * n + j) != stamp {
                            continue;
                        }
                        changed += 1;
                        let i = y0 * W() + j;
                        let (c, f, t) = (word(cells_at + j), word(cells_at + n + j), f32::from_bits(word(cells_at + 2 * n + j)));
                        let known = Mat::from_u8((cu[x] & 0xff) as u8);
                        let (known_f, known_t) = (fu[x], tu[x]);
                        (cu[x], fu[x], tu[x]) = (c, f, t);
                        if audit {
                            let water = |m: Mat, q: u32| if m == Mat::Water { q as i64 } else { 0 };
                            AUDIT[0].fetch_add(water(Mat::from_u8((c & 0xff) as u8), f) - water(known, known_f), std::sync::atomic::Ordering::Relaxed);
                            if wc[x].m == known && known == Mat::Water {
                                let have = match wf[x] {
                                    0 => field::FULL,
                                    q => q,
                                };
                                if have != known_f {
                                    AUDIT[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    AUDIT[2].fetch_add(have as i64 - known_f as i64, std::sync::atomic::Ordering::Relaxed);
                                }
                            } else if wc[x].m != known {
                                if AUDIT[3].load(std::sync::atomic::Ordering::Relaxed) < 3 {
                                    eprintln!("    ({x}, {}) : la carte connaissait {known:?} {known_f}, le CPU a {:?} {} (posée au pas {}, image {frame}), la carte dit {:?} {f}", y0 + yy, wc[x].m, wf[x], wc[x].stamp, Mat::from_u8((c & 0xff) as u8));
                                }
                                AUDIT[3].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                AUDIT[4].fetch_add(water(known, known_f) - water(Mat::from_u8((c & 0xff) as u8), f), std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                        if wc[x].m != known {
                            continue;
                        }
                        let m = Mat::from_u8((c & 0xff) as u8);
                        let (f, t) = if m == known && m.kind() == Kind::Liquid {
                            let have = match wf[x] {
                                0 => field::FULL,
                                q => q,
                            };
                            ((have as i64 + f as i64 - known_f as i64).max(0) as u32, wt[x] + (t - known_t))
                        } else {
                            (f, t)
                        };
                        let m = if m.kind() == Kind::Liquid && f <= CRUMB { Mat::Empty } else { m };
                        let fill = if f == field::FULL { 0 } else { f };
                        if m != wc[x].m {
                            wc[x] = Cell { m, stamp: frame };
                            // (Its states are cleared after, which blanks the fill: it is set again.)
                            cleared.push((i, if m.kind() == Kind::Liquid { fill } else { 0 }));
                        }
                        if m.kind() == Kind::Liquid {
                            wt[x] = t;
                            wf[x] = fill;
                        }
                        // One wake per zone a row crosses (and the cells on a zone's edge wake
                        // the zone next door: those are woken one by one).
                        let zone = x / sleep::CS;
                        if zone != last_zone || x % sleep::CS == 0 || x % sleep::CS == sleep::CS - 1 || (y0 + yy) % sleep::CS == 0 || (y0 + yy) % sleep::CS == sleep::CS - 1 {
                            woken.push(i);
                            last_zone = zone;
                        }
                    }
                    (changed, woken, cleared)
                })
                .reduce(|| (0, Vec::new(), Vec::new()), |mut a, mut b| {
                    a.1.append(&mut b.1);
                    a.2.append(&mut b.2);
                    (a.0 + b.0, a.1, a.2)
                });
            DBG[7].store(changed as u64, std::sync::atomic::Ordering::Relaxed);
            for (i, fill) in cleared {
                self.clear_states(i);
                self.fill[i] = fill;
                self.latent[i] = 0.0;
                self.vel[i] = 0;
            }
            for i in woken {
                self.zones.wake(i);
            }
            if changed > 0 {
                self.revision += 1;
            }
            drop(data);
            card.read.unmap();
            // The drops thrown off (in a fixed order, so that a run is the same twice).
            throws.sort_by_key(|t| t.cell);
            for t in throws {
                let jitter = |r: u32| (r % 1000) as f32 / 1000.0 - 0.5;
                let (jx, jy) = (jitter(self.rand()), jitter(self.rand()));
                liq.drops.push(Drop { x: t.x, y: t.y, vx: t.vx + jx, vy: t.vy + jy, m: Mat::from_u8(t.mat as u8), q: t.q, t: t.temp });
            }
            tick(6, t);
            if audit {
                let after = units(self) + liq.drops.iter().filter(|d| d.m == Mat::Water).map(|d| d.q as u64).sum::<u64>();
                let a = |k: usize| AUDIT[k].swap(0, std::sync::atomic::Ordering::Relaxed);
                let (card_delta, stale_n, stale_q, other_n, other_q) = (a(0), a(1), a(2), a(3), a(4));
                let thrown: i64 = liq.drops.iter().rev().take(count).filter(|d| d.m == Mat::Water).map(|d| d.q as i64).sum();
                if after != before || card_delta + thrown != 0 {
                    eprintln!("pas {}: l'eau passe de {before} à {after} ({:+}) en reprenant la carte ({} gouttes lancées, {} cellules changées ; la carte a déplacé {:+} et jeté {thrown}, {stale_n} cellules changées entre-temps sur le CPU de {stale_q:+}, {other_n} cellules dont la matière a changé, ignorant {other_q:+})", pend.stamp, after as i64 - before as i64, count, changed, card_delta);
                }
            }
            self.prof.stop(prof::LIQ_DOWN, clock);
        }

        /// The step given to the card: the cells the game changed go to it, and the whole turn
        /// is encoded and submitted, its result to be read at the next step.
        fn submit_liquids(&mut self, liq: &mut Liquids, card: &mut LiquidGpu, step: f32) {
            let clock = self.prof.start();
            let mut t = std::time::Instant::now();
            // The band: the rows that hold liquid in zones that run, a margin, what was sent.
            let awake = &self.zones.awake;
            let liquid_rows: Vec<bool> = (0..H())
                .into_par_iter()
                .map(|y| (0..W()).any(|x| self.cells[y * W() + x].m.kind() == Kind::Liquid && awake[y / sleep::CS * sleep::CW() + x / sleep::CS]))
                .collect();
            let first = liquid_rows.iter().position(|&r| r);
            let last = liquid_rows.iter().rposition(|&r| r);
            let margin = max_sub(step) + 3;
            let now = first.map(|a| (a.saturating_sub(margin), (last.unwrap() + margin).min(H() - 1)));
            let band = match (now, card.sent) {
                (Some((a, b)), Some((c, d))) => (a.min(c), b.max(d)),
                (Some(ab), None) | (None, Some(ab)) => Some(ab).unwrap(),
                (None, None) => {
                    self.prof.stop(prof::LIQ_UP, clock);
                    return;
                }
            };
            card.sent = now;
            card.stamp += 1;
            let stamp = card.stamp;
            let (y0, y1) = band;
            let rows = y1 + 1 - y0;
            let cells = y0 * W()..(y1 + 1) * W();
            // The cells as the game has them now, and what the step needs to know of them: the
            // full cells that run (the size of the pressure's system), whether any is thick, the
            // cells that fell (their speed to the edges around).
            let (mut code_v, mut fill_v, mut temp_v, mut gaspa_v) = (std::mem::take(&mut card.code_new), std::mem::take(&mut card.fill_new), std::mem::take(&mut card.temp_new), std::mem::take(&mut card.gaspa_new));
            let (code, fill, temp, gaspa) = (&mut code_v, &mut fill_v, &mut temp_v, &mut gaspa_v);
            let world: &World = self;
            let (fullish, thick, fallen): (usize, bool, Vec<(usize, f32)>) = code[cells.clone()]
                .par_chunks_mut(W())
                .zip(fill[cells.clone()].par_chunks_mut(W()))
                .zip(temp[cells.clone()].par_chunks_mut(W()))
                .zip(gaspa[cells.clone()].par_chunks_mut(W()))
                .enumerate()
                .map(|(yy, (((code, fill), temp), gaspa))| {
                    let y = y0 + yy;
                    let (mut fullish, mut thick, mut fallen) = (0, false, Vec::new());
                    for x in 0..W() {
                        let i = y * W() + x;
                        let m = world.cells[i].m;
                        let run = awake[y / sleep::CS * sleep::CW() + x / sleep::CS];
                        let mut c = m as u32;
                        if world.mask[i] != 0 {
                            c |= MASKED;
                        }
                        if run {
                            c |= AWAKE;
                        }
                        let pocket = world.pocket_at(x as i32, y as i32);
                        if m.kind() == Kind::Gas && world.gas_open(x as i32, y as i32) {
                            c |= OPEN;
                        }
                        code[x] = c;
                        gaspa[x] = pocket.map_or(0.0, |k| (world.pockets[k].pressure - 1.0) * ATM + world.surge_pa(x as i32, y as i32));
                        temp[x] = world.temp[i];
                        if m.kind() == Kind::Liquid {
                            let f = world.liquid_fill(i);
                            fill[x] = f;
                            if run && world.mask[i] == 0 {
                                fullish += (f >= field::FULL / HOLDS) as usize;
                                thick |= m == Mat::Lava;
                            }
                            if world.vel[i] != 0 && !world.lone_liquid(x as i32, y as i32) {
                                fallen.push((i, world.vel[i] as f32 / FALL_UNIT as f32 * cell_m() / DT));
                            }
                        } else {
                            fill[x] = 0;
                        }
                    }
                    (fullish, thick, fallen)
                })
                .reduce(|| (0, false, Vec::new()), |mut a, mut b| {
                    a.2.append(&mut b.2);
                    (a.0 + b.0, a.1 || b.1, a.2)
                });
            // What the card holds that is not what the game has: those cells go, and no other.
            t = tick(0, t);
            let (held_code, held_fill, held_temp, held_gaspa) = (&card.code_up, &card.fill_up, &card.temp_up, &card.gaspa_up);
            let (code, fill, temp, gaspa) = (&code_v, &fill_v, &temp_v, &gaspa_v);
            let held = card.held;
            let sent: Vec<CellRecord> = (y0..=y1)
                .into_par_iter()
                .flat_map_iter(|y| {
                    let fresh = held.is_none_or(|(a, b)| y < a || y > b);
                    (y * W()..(y + 1) * W()).filter_map(move |i| {
                        // (A little heat or pocket pressure makes no difference to the liquid: the
                        // whole pool would go up every third step, when the heat spreads, for
                        // hundredths of a degree. The card keeps what it has until the gap counts.)
                        if fresh || code[i] != held_code[i] || fill[i] != held_fill[i] || (temp[i] - held_temp[i]).abs() > TEMP_GAP || (gaspa[i] - held_gaspa[i]).abs() > GASPA_GAP {
                            Some(CellRecord { cell: i as u32, code: code[i], fill: fill[i], temp: temp[i], gaspa: gaspa[i] })
                        } else {
                            None
                        }
                    })
                })
                .collect();
            card.held = Some(match held {
                Some((a, b)) => (a.min(y0), b.max(y1)),
                None => (y0, y1),
            });
            // A drop that landed gives its speed to the edges under and over it.
            t = tick(1, t);
            let mut patches = std::mem::take(&mut card.patches_up);
            let mut fastest = card.fastest;
            for (i, speed) in fallen {
                let (x, y) = (i % W(), i / W());
                for e in [vi(x, y + 1), vi(x, y)] {
                    patches.push(Patch { edge: e as u32 | 0x8000_0000, mode: 0, value: speed, weight: 0.0 });
                }
                self.vel[i] = 0;
            }
            for pt in &patches {
                fastest = fastest.max(pt.value.abs());
            }
            patches.truncate(MAX_PATCHES);
            let (turns, per_turn) = turns(step, fastest, self.liquid_sub_steps());
            let sub = turns * per_turn;
            let dt = step / sub as f32;
            // To the card.
            let q = &card.gpu.queue;
            let at = |c: usize| (c * 4) as u64;
            if !sent.is_empty() {
                q.write_buffer(&card.sent_cells, 0, bytes(&sent));
            }
            if !patches.is_empty() {
                q.write_buffer(&card.patches, 0, bytes(&patches));
            }
            q.write_buffer(&card.throws_n, 0, bytes(&[0u32]));
            q.write_buffer(&card.tables, 256 * 4, bytes(self.flow.skim_table()));
            q.write_buffer(&card.tables, (256 + flow::FW() * flow::FH()) as u64 * 4, bytes(self.flow.sky_table()));
            let groups = (rows * W()).div_ceil(WG);
            for colour in 0..2u32 {
                let pr = Params { w: W() as u32, h: H() as u32, y0: y0 as u32, rows: rows as u32, dt, colour, patches: patches.len() as u32, groups: groups as u32, stamp, sent: sent.len() as u32, time: self.ticks as f32 * DT, pad: 0 };
                q.write_buffer(&card.params, colour as u64 * SLOT, bytes(&[pr]));
            }
            // The game's cells are what the card holds now.
            // What the card holds now: what was sent, and for the rest what it held.
            for r in &sent {
                let i = r.cell as usize;
                (card.code_up[i], card.fill_up[i], card.temp_up[i], card.gaspa_up[i]) = (r.code, r.fill, r.temp, r.gaspa);
            }

            (card.code_new, card.fill_new, card.temp_new, card.gaspa_new) = (code_v, fill_v, temp_v, gaspa_v);
            t = tick(2, t);
            self.prof.stop(prof::LIQ_UP, clock);
            let clock = self.prof.start();
            // The sub-steps.
            let k = &card.kernels;
            let ncell = rows * W();
            let (nu_, nv) = (rows * (W() + 1), rows * W());
            let (ubytes, vbytes) = (at(y0 * (W() + 1))..at((y1 + 1) * (W() + 1)), at(y0 * W())..at((y1 + 1) * W()));
            let mut enc = card.encoder();
            // The time stamps, where asked for: one at the start, one at the end of each phase
            // (and inside the pressure's solve, its parts).
            let mut marks: Vec<&'static str> = Vec::new();
            let mut stamps = Stamper { set: card.stamps.as_ref().map(|(set, _)| set), marks: &mut marks, max: MAX_STAMPS };
            let stamp_at = |enc: &mut wgpu::CommandEncoder, stamps: &mut Stamper, what: &'static str| stamps.at(enc, what);
            stamp_at(&mut enc, &mut stamps, "début");
            {
                let mut pass = Pass::new(&mut enc);
                if !sent.is_empty() {
                    card.run(&mut pass, &k.apply_cells, 0, sent.len());
                }
                if !patches.is_empty() {
                    card.run(&mut pass, &k.patch, 0, patches.len());
                }
            }
            stamp_at(&mut enc, &mut stamps, "envoi");
            for s in 0..sub {
                {
                    let mut pass = Pass::new(&mut enc);
                    card.run(&mut pass, &k.classify, 0, ncell);
                    card.run(&mut pass, &k.forward_u, 0, nu_);
                    card.run(&mut pass, &k.forward_v, 0, nv);
                    card.run(&mut pass, &k.back_u, 0, nu_);
                    card.run(&mut pass, &k.back_v, 0, nv);
                }
                enc.copy_buffer_to_buffer(&card.u3, ubytes.start, &card.u, ubytes.start, ubytes.end - ubytes.start);
                enc.copy_buffer_to_buffer(&card.v3, vbytes.start, &card.v, vbytes.start, vbytes.end - vbytes.start);
                stamp_at(&mut enc, &mut stamps, "forces");
                if thick {
                    {
                        let mut pass = Pass::new(&mut enc);
                        card.run(&mut pass, &k.thickness, 0, ncell);
                    }
                    enc.copy_buffer_to_buffer(&card.u, ubytes.start, &card.u2, ubytes.start, ubytes.end - ubytes.start);
                    enc.copy_buffer_to_buffer(&card.v, vbytes.start, &card.v2, vbytes.start, vbytes.end - vbytes.start);
                    let mut pass = Pass::new(&mut enc);
                    for _ in 0..VISCOUS_SWEEPS {
                        for colour in 0..2 {
                            card.run(&mut pass, &k.viscous_u, colour, nu_);
                        }
                    }
                    for _ in 0..VISCOUS_SWEEPS {
                        for colour in 0..2 {
                            card.run(&mut pass, &k.viscous_v, colour, nv);
                        }
                    }
                    drop(pass);
                    stamp_at(&mut enc, &mut stamps, "viscosité");
                }
                if (s + 1) % per_turn == 0 {
                    {
                        let mut pass = Pass::new(&mut enc);
                        card.run(&mut pass, &k.system_cells, 0, ncell);
                        card.run(&mut pass, &k.system_edges, 0, ncell);
                        card.run(&mut pass, &k.assemble, 0, ncell);
                    }
                    card.pressure.solve_fixed(&mut enc, y0, y1, fullish, SETTLED, RELATIVE, card.iters, &mut stamps);
                    let mut pass = Pass::new(&mut enc);
                    card.run(&mut pass, &k.take_pressure, 0, ncell);
                    card.run(&mut pass, &k.push_u, 0, nu_);
                    card.run(&mut pass, &k.push_v, 0, nv);
                    drop(pass);
                    stamp_at(&mut enc, &mut stamps, "pression");
                }
                let mut pass = Pass::new(&mut enc);
                card.run(&mut pass, &k.throw_find, 0, ncell);
                card.run(&mut pass, &k.throw_apply, 0, ncell);
                card.run(&mut pass, &k.carry_flow, 0, ncell);
                card.run(&mut pass, &k.carry_want, 0, ncell);
                card.run(&mut pass, &k.carry_scale, 0, ncell);
                card.run(&mut pass, &k.carry_settle, 0, ncell);
                card.run(&mut pass, &k.carry_gather, 0, ncell);
                card.run(&mut pass, &k.carry_commit, 0, ncell);
                card.run(&mut pass, &k.overflow, 0, W());
                drop(pass);
                stamp_at(&mut enc, &mut stamps, "transport");
            }
            {
                let mut pass = Pass::new(&mut enc);
                card.run(&mut pass, &k.fastest, 0, ncell);
            }
            // Back: the band's class and pressure, the throws, the changed cells, the fastest
            // edge, the pressure's scalars. The card is not waited for: the result is read at
            // the next step.
            let n = rows * W();
            let mut off = 0u64;
            for buf in [&card.class, &card.p] {
                enc.copy_buffer_to_buffer(buf, at(cells.start), &card.read, off, at(n));
                off += at(n);
            }
            enc.copy_buffer_to_buffer(&card.throws_n, 0, &card.read, off, 4);
            off += 4;
            let throws_at = off as usize / 4;
            enc.copy_buffer_to_buffer(&card.throws, 0, &card.read, off, (MAX_THROWS * 32) as u64);
            off += (MAX_THROWS * 32) as u64;
            let cells_at = off as usize / 4;
            for buf in [&card.code, &card.fill, &card.temp, &card.mark] {
                enc.copy_buffer_to_buffer(buf, at(cells.start), &card.read, off, at(n));
                off += at(n);
            }
            let partials_at = off as usize / 4;
            enc.copy_buffer_to_buffer(&card.partials, 0, &card.read, off, at(groups));
            off += at(groups);
            let sc_at = off as usize / 4;
            enc.copy_buffer_to_buffer(card.pressure.scalars(), 0, &card.read, off, (SC_N * 4) as u64);
            off += (SC_N * 4) as u64;
            // The copies back are timed too, then the stamps are resolved and copied last.
            stamp_at(&mut enc, &mut stamps, "retour");
            let stamps_at = off as usize / 4;
            drop(stamps);
            if let Some((set, buf)) = &card.stamps {
                if marks.len() > 1 {
                    enc.resolve_query_set(set, 0..marks.len() as u32, buf, 0);
                    enc.copy_buffer_to_buffer(buf, 0, &card.read, off, (marks.len() * 8) as u64);
                }
                off += (MAX_STAMPS * 8) as u64;
            }
            let total = off;
            t = tick(3, t);
            card.gpu.queue.submit([enc.finish()]);
            card.read.slice(..total).map_async(wgpu::MapMode::Read, |_| {});
            t = tick(4, t);
            card.pending = Some(Pending { y0, rows, groups, throws_at, cells_at, partials_at, stamp, sc_at, stamps_at, marks, total });
            if crate::config::config().debug_card && stamp % 100 == 0 {
                let ms = |k: usize| DBG[k].load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e6 / stamp as f64;
                eprintln!("carte, ms/pas: bande {:.2}, différences {:.2} ({} cellules envoyées), écritures {:.2}, encodage {:.2}, soumission {:.2}, attente {:.2}, lecture {:.2} ({} cellules reçues)", ms(0), ms(1), sent.len(), ms(2), ms(3), ms(4), ms(5), ms(6), DBG[7].load(std::sync::atomic::Ordering::Relaxed));
                if card.stamps.is_some() {
                    let line: Vec<String> = PHASES.iter().enumerate().map(|(k, name)| format!("{name} {:.2}", CARD[k].load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e6 / stamp as f64)).collect();
                    eprintln!("carte, ms/pas sur la carte: {} ({} lignes, {} sous-pas, {} itérations, {:.2} en moyenne)", line.join(", "), rows, sub, card.iters, DBG_ITER.load(std::sync::atomic::Ordering::Relaxed) as f64 / stamp as f64);
                } else {
                    eprintln!("carte: pas d'horodatage (la carte ne l'offre pas)");
                }
            }
            let _ = t;
            liq.solve = (None, 0);
            self.prof.stop(prof::LIQ_UP, clock);
        }
    }
}
