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
pub struct LiquidGpu;

#[cfg(target_arch = "wasm32")]
impl LiquidGpu {
    pub fn new() -> Option<Self> {
        None
    }
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
    use super::super::mgpcg_gpu::PressureGpu;
    use super::super::*;
    use super::Patch;
    use rayon::prelude::*;

    const WG: usize = 256;
    const SLOT: u64 = 256;
    const MAX_THROWS: usize = 16384;
    const MAX_PATCHES: usize = 1 << 16;
    const MAX_CHANGES: usize = 1 << 18;
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
        pad: [u32; 2],
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

    struct Kernels {
        apply_cells: wgpu::ComputePipeline,
        collect_changes: wgpu::ComputePipeline,
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
        gaspa: wgpu::Buffer,
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
        changes_n: wgpu::Buffer,
        records: wgpu::Buffer,
        /// What comes back: the band's class and p, the throws, the changed cells, the partials.
        read: wgpu::Buffer,
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
        /// The patches to apply at the start of the next step.
        pub patches_up: Vec<Patch>,
    }

    impl LiquidGpu {
        /// The liquids' turn on the card, if the game has one.
        pub fn new() -> Option<Self> {
            let gpu = gpu()?;
            let pressure = PressureGpu::new(W, H)?;
            let d = &gpu.device;
            assert!(Mat::Water as u8 == 2 && Mat::Stone as u8 == 7 && Mat::Empty as u8 == 0, "liquid.wgsl names these materials by their number");
            let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("liquid"), source: wgpu::ShaderSource::Wgsl(include_str!("liquid.wgsl").into()) });
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
                d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("liquid edges"), entries: &(0..13).map(storage).collect::<Vec<_>>() }),
            ];
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("liquid"), bind_group_layouts: &[&layouts[0], &layouts[1], &layouts[2], &layouts[3]], push_constant_ranges: &[] });
            let pipeline = |entry: &str| {
                d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: Some(&layout), module: &shader, entry_point: Some(entry), compilation_options: Default::default(), cache: None })
            };
            let kernels = Kernels {
                apply_cells: pipeline("apply_cells"),
                collect_changes: pipeline("collect_changes"),
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
            let n = W * H;
            let (nu_, nv) = ((W + 1) * H, W * (H + 1));
            let params = d.create_buffer(&wgpu::BufferDescriptor { label: Some("liquid params"), size: SLOT * 2, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let (code, fill, temp, gaspa, class, p, nu, rhs) = (grid("code", n), grid("fill", n), grid("temp", n), grid("gaspa", n), grid("class", n), grid("p", n), grid("nu", n), grid("rhs", n));
            let tables = grid("tables", 256);
            let (want, scale, qnew, tnew, fu, fv, thrown, mnew) = (grid("want", n), grid("scale", n), grid("qnew", n), grid("tnew", n), grid("fu", nu_), grid("fv", nv), grid("thrown", n), grid("mnew", n));
            let groups = n.div_ceil(WG);
            let partials = grid("partials", groups);
            let mark = grid("mark", n);
            let (u, v, u2, v2, u3, v3) = (grid("u", nu_), grid("v", nv), grid("u2", nu_), grid("v2", nv), grid("u3", nu_), grid("v3", nv));
            let patches = grid("patches", MAX_PATCHES * 4);
            let throws_n = grid("throws n", 1);
            let throws = grid("throws", MAX_THROWS * 8);
            let sent_cells = grid("sent cells", n * 5);
            let changes_n = grid("changes n", 1);
            let changes = grid("changes", MAX_CHANGES);
            let records = grid("records", MAX_CHANGES * 5);
            let read_size = (2 * n + 1 + MAX_THROWS * 8 + 1 + MAX_CHANGES * 5 + groups) * 4;
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
                d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("liquid edges"), layout: &layouts[3], entries: &entries(&[&u, &v, &u2, &v2, &u3, &v3, &patches, &throws_n, &throws, &sent_cells, &changes_n, &changes, &records]) }),
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
                gaspa,
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
                changes_n,
                records,
                read,
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
                patches_up: Vec::new(),
            })
        }

        fn encoder(&self) -> wgpu::CommandEncoder {
            self.gpu.device.create_command_encoder(&Default::default())
        }

        /// One kernel over `n` threads, with the parameters of `colour`.
        fn run(&self, pass: &mut wgpu::ComputePass, pipeline: &wgpu::ComputePipeline, colour: u32, n: usize) {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &self.binds[0], &[colour * SLOT as u32]);
            pass.set_bind_group(1, &self.binds[1], &[]);
            pass.set_bind_group(2, &self.binds[2], &[]);
            pass.set_bind_group(3, &self.binds[3], &[]);
            pass.dispatch_workgroups(n.div_ceil(WG) as u32, 1, 1);
        }
    }

    impl World {
        /// The liquids' turn on the card: `liq` has had its drops flown (`fly_drops`) already.
        pub(in super::super) fn step_liquids_on_card(&mut self, liq: &mut Liquids, card: &mut LiquidGpu) {
            let clock = self.prof.start();
            // The band: the rows that hold liquid in zones that run, a margin, what was sent.
            let awake = &self.zones.awake;
            let liquid_rows: Vec<bool> = (0..H)
                .into_par_iter()
                .map(|y| (0..W).any(|x| self.cells[y * W + x].m.kind() == Kind::Liquid && awake[y / sleep::CS * sleep::CW + x / sleep::CS]))
                .collect();
            let first = liquid_rows.iter().position(|&r| r);
            let last = liquid_rows.iter().rposition(|&r| r);
            let margin = MAX_SUB + 3;
            let now = first.map(|a| (a.saturating_sub(margin), (last.unwrap() + margin).min(H - 1)));
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
            let cells = y0 * W..(y1 + 1) * W;
            // The cells as the game has them now, and what the step needs to know of them: the
            // full cells that run (the size of the pressure's system), whether any is thick, the
            // cells that fell (their speed to the edges around).
            let (mut code_v, mut fill_v, mut temp_v, mut gaspa_v) = (std::mem::take(&mut card.code_new), std::mem::take(&mut card.fill_new), std::mem::take(&mut card.temp_new), std::mem::take(&mut card.gaspa_new));
            let (code, fill, temp, gaspa) = (&mut code_v, &mut fill_v, &mut temp_v, &mut gaspa_v);
            let world: &World = self;
            let (fullish, thick, fallen): (usize, bool, Vec<(usize, f32)>) = code[cells.clone()]
                .par_chunks_mut(W)
                .zip(fill[cells.clone()].par_chunks_mut(W))
                .zip(temp[cells.clone()].par_chunks_mut(W))
                .zip(gaspa[cells.clone()].par_chunks_mut(W))
                .enumerate()
                .map(|(yy, (((code, fill), temp), gaspa))| {
                    let y = y0 + yy;
                    let (mut fullish, mut thick, mut fallen) = (0, false, Vec::new());
                    for x in 0..W {
                        let i = y * W + x;
                        let m = world.cells[i].m;
                        let run = awake[y / sleep::CS * sleep::CW + x / sleep::CS];
                        let mut c = m as u32;
                        if world.mask[i] != 0 {
                            c |= MASKED;
                        }
                        if run {
                            c |= AWAKE;
                        }
                        let pocket = world.pocket_at(x as i32, y as i32);
                        if m.kind() == Kind::Gas && pocket.is_some() {
                            c |= OPEN;
                        }
                        code[x] = c;
                        gaspa[x] = pocket.map_or(0.0, |k| (world.pockets[k].pressure - 1.0) * ATM);
                        temp[x] = world.temp[i];
                        if m.kind() == Kind::Liquid {
                            let f = world.liquid_fill(i);
                            fill[x] = f;
                            if run && world.mask[i] == 0 {
                                fullish += (f >= field::FULL / HOLDS) as usize;
                                thick |= m == Mat::Lava;
                            }
                            if world.vel[i] != 0 && !world.lone_liquid(x as i32, y as i32) {
                                fallen.push((i, world.vel[i] as f32 / 8.0 * CELL_M / DT));
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
            let (held_code, held_fill, held_temp, held_gaspa) = (&card.code_up, &card.fill_up, &card.temp_up, &card.gaspa_up);
            let (code, fill, temp, gaspa) = (&code_v, &fill_v, &temp_v, &gaspa_v);
            let held = card.held;
            let sent: Vec<CellRecord> = (y0..=y1)
                .into_par_iter()
                .flat_map_iter(|y| {
                    let fresh = held.is_none_or(|(a, b)| y < a || y > b);
                    (y * W..(y + 1) * W).filter_map(move |i| {
                        if fresh || code[i] != held_code[i] || fill[i] != held_fill[i] || temp[i].to_bits() != held_temp[i].to_bits() || gaspa[i].to_bits() != held_gaspa[i].to_bits() {
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
            let mut patches = std::mem::take(&mut card.patches_up);
            let mut fastest = card.fastest;
            for (i, speed) in fallen {
                let (x, y) = (i % W, i / W);
                for e in [vi(x, y + 1), vi(x, y)] {
                    patches.push(Patch { edge: e as u32 | 0x8000_0000, mode: 0, value: speed, weight: 0.0 });
                }
                self.vel[i] = 0;
            }
            for pt in &patches {
                fastest = fastest.max(pt.value.abs());
            }
            patches.truncate(MAX_PATCHES);
            let sub = (((fastest + G * DT) * DT / CELL_M).ceil() as usize).clamp(1, MAX_SUB);
            let dt = DT / sub as f32;
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
            q.write_buffer(&card.changes_n, 0, bytes(&[0u32]));
            let groups = (rows * W).div_ceil(WG);
            for colour in 0..2u32 {
                let pr = Params { w: W as u32, h: H as u32, y0: y0 as u32, rows: rows as u32, dt, colour, patches: patches.len() as u32, groups: groups as u32, stamp, sent: sent.len() as u32, pad: [0; 2] };
                q.write_buffer(&card.params, colour as u64 * SLOT, bytes(&[pr]));
            }
            // The game's cells are what the card holds now.
            std::mem::swap(&mut card.code_up, &mut code_v);
            std::mem::swap(&mut card.fill_up, &mut fill_v);
            std::mem::swap(&mut card.temp_up, &mut temp_v);
            std::mem::swap(&mut card.gaspa_up, &mut gaspa_v);
            (card.code_new, card.fill_new, card.temp_new, card.gaspa_new) = (code_v, fill_v, temp_v, gaspa_v);
            self.prof.stop(prof::LIQ_UP, clock);
            let clock = self.prof.start();
            // The sub-steps.
            let k = &card.kernels;
            let ncell = rows * W;
            let (nu_, nv) = (rows * (W + 1), rows * W);
            let (ubytes, vbytes) = (at(y0 * (W + 1))..at((y1 + 1) * (W + 1)), at(y0 * W)..at((y1 + 1) * W));
            let mut enc = card.encoder();
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                if !sent.is_empty() {
                    card.run(&mut pass, &k.apply_cells, 0, sent.len());
                }
                if !patches.is_empty() {
                    card.run(&mut pass, &k.patch, 0, patches.len());
                }
            }
            let mut iterations = 0;
            for s in 0..sub {
                {
                    let mut pass = enc.begin_compute_pass(&Default::default());
                    card.run(&mut pass, &k.classify, 0, ncell);
                    card.run(&mut pass, &k.forward_u, 0, nu_);
                    card.run(&mut pass, &k.forward_v, 0, nv);
                    card.run(&mut pass, &k.back_u, 0, nu_);
                    card.run(&mut pass, &k.back_v, 0, nv);
                }
                enc.copy_buffer_to_buffer(&card.u3, ubytes.start, &card.u, ubytes.start, ubytes.end - ubytes.start);
                enc.copy_buffer_to_buffer(&card.v3, vbytes.start, &card.v, vbytes.start, vbytes.end - vbytes.start);
                if thick {
                    {
                        let mut pass = enc.begin_compute_pass(&Default::default());
                        card.run(&mut pass, &k.thickness, 0, ncell);
                    }
                    enc.copy_buffer_to_buffer(&card.u, ubytes.start, &card.u2, ubytes.start, ubytes.end - ubytes.start);
                    enc.copy_buffer_to_buffer(&card.v, vbytes.start, &card.v2, vbytes.start, vbytes.end - vbytes.start);
                    let mut pass = enc.begin_compute_pass(&Default::default());
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
                }
                if s + 1 == sub {
                    {
                        let mut pass = enc.begin_compute_pass(&Default::default());
                        card.run(&mut pass, &k.system_cells, 0, ncell);
                        card.run(&mut pass, &k.system_edges, 0, ncell);
                        card.run(&mut pass, &k.assemble, 0, ncell);
                    }
                    let solve_clock = self.prof.start();
                    let (done, after) = card.pressure.solve_resident(enc, y0, y1, fullish, SETTLED, RELATIVE, MAX_ITER);
                    self.prof.stop_n(prof::LIQ_ITERATIONS, solve_clock, done);
                    iterations = done;
                    enc = after;
                    let mut pass = enc.begin_compute_pass(&Default::default());
                    card.run(&mut pass, &k.take_pressure, 0, ncell);
                    card.run(&mut pass, &k.push_u, 0, nu_);
                    card.run(&mut pass, &k.push_v, 0, nv);
                }
                let mut pass = enc.begin_compute_pass(&Default::default());
                card.run(&mut pass, &k.throw_find, 0, ncell);
                card.run(&mut pass, &k.throw_apply, 0, ncell);
                card.run(&mut pass, &k.carry_flow, 0, ncell);
                card.run(&mut pass, &k.carry_want, 0, ncell);
                card.run(&mut pass, &k.carry_scale, 0, ncell);
                card.run(&mut pass, &k.carry_settle, 0, ncell);
                card.run(&mut pass, &k.carry_gather, 0, ncell);
                card.run(&mut pass, &k.carry_commit, 0, ncell);
                card.run(&mut pass, &k.overflow, 0, W);
            }
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                card.run(&mut pass, &k.fastest, 0, ncell);
                card.run(&mut pass, &k.collect_changes, 0, MAX_CHANGES);
            }
            // Back: the band's class and pressure, the throws, the changed cells, the fastest edge.
            let n = rows * W;
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
            let changes_at = off as usize / 4;
            enc.copy_buffer_to_buffer(&card.changes_n, 0, &card.read, off, 4);
            off += 4;
            let records_at = off as usize / 4;
            enc.copy_buffer_to_buffer(&card.records, 0, &card.read, off, (MAX_CHANGES * 20) as u64);
            off += (MAX_CHANGES * 20) as u64;
            let partials_at = off as usize / 4;
            enc.copy_buffer_to_buffer(&card.partials, 0, &card.read, off, at(groups));
            let total = off + at(groups);
            card.gpu.queue.submit([enc.finish()]);
            let slice = card.read.slice(..total);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            card.gpu.device.poll(wgpu::Maintain::Wait);
            self.prof.stop(prof::LIQ_CARD, clock);
            let clock = self.prof.start();
            let data = slice.get_mapped_range();
            let data_ref: &[u8] = &data;
            let word = |k: usize| u32::from_le_bytes([data_ref[k * 4], data_ref[k * 4 + 1], data_ref[k * 4 + 2], data_ref[k * 4 + 3]]);
            let count = (word(throws_at - 1) as usize).min(MAX_THROWS);
            let mut throws: Vec<Throw> = (0..count)
                .map(|t| {
                    let k = throws_at + t * 8;
                    Throw { cell: word(k), q: word(k + 1), x: f32::from_bits(word(k + 2)), y: f32::from_bits(word(k + 3)), vx: f32::from_bits(word(k + 4)), vy: f32::from_bits(word(k + 5)), mat: word(k + 6), temp: f32::from_bits(word(k + 7)) }
                })
                .collect();
            let changed = (word(changes_at) as usize).min(MAX_CHANGES);
            debug_assert!(changed < MAX_CHANGES, "the card changed more cells than it can report");
            card.fastest = (0..groups).map(|g| f32::from_bits(word(partials_at + g))).fold(0.0f32, f32::max);
            liq.class[cells.clone()].par_iter_mut().enumerate().for_each(|(j, c)| *c = word(j) as u8);
            liq.p[cells.clone()].par_iter_mut().enumerate().for_each(|(j, c)| *c = f32::from_bits(word(n + j)));
            // What the card changed, into the cells (and into what the card is known to hold).
            for r in 0..changed {
                let k = records_at + r * 5;
                let (i, c, f, t) = (word(k) as usize, word(k + 1), word(k + 2), f32::from_bits(word(k + 3)));
                card.code_up[i] = c;
                card.fill_up[i] = f;
                card.temp_up[i] = t;
                let m = Mat::from_u8((c & 0xff) as u8);
                if m != self.cells[i].m {
                    self.cells[i] = Cell { m, stamp: self.frame };
                    self.clear_states(i);
                    self.latent[i] = 0.0;
                    self.vel[i] = 0;
                }
                if m.kind() == Kind::Liquid {
                    self.temp[i] = t;
                    self.fill[i] = if f == field::FULL { 0 } else { f };
                }
                self.revision += 1;
                self.zones.wake(i);
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
            liq.solve = (None, iterations);
            self.prof.stop(prof::LIQ_DOWN, clock);
        }
    }
}
