# Performance audit: i7-8550U + UHD 620

Measured on 2026-10-29 with the built-in bench (`cargo test --release --bin aworld bench_scenes -- --ignored --nocapture`)
and a `--demo` run with `AWORLD_PROFILE=1`. Machine: Intel i7-8550U (4 cores / 8 threads), Intel UHD 620 iGPU.
Budget: 16.7 ms per step at 60 fps.

## Results

| Scene | ms/step | worst step | Liquids' share |
|---|---|---|---|
| démo | 8 | 22 | 7 % |
| incendie | 10 | 33 | 7 % |
| orage | 30 | 70 | 52 % |
| nuage | 60 | 132 | 71 % |
| liquides | 61 | 99 | 78 % |
| déluge | 109 | 145 | 85 % |

## Findings

1. **Liquid physics is the bottleneck.** Any scene with liquid in motion is 4–6× over budget. Heat, air, cells and
   structures stay small (a `structures` spike of 10–20 ms every 30 steps aside).
2. **Cost is spread over `forces`, `transport`, `pression`, `itérations`.** In *liquides*, forces + transport are about
   50 % and the pressure solver about 20 %. Forces and transport run once per sub-step, and the scene takes 3.5–4
   sub-steps per step (cap `MAX_SUB` = 4), so the cost is nearly proportional to the sub-step count.
3. **More threads do not help.** Liquid time: 142 ms on 1 thread, 59 ms on 4, 50 ms on 8.
4. **The iGPU does not help.** `AWORLD_GPU=auto`: card kernels take 44 ms per step plus about 8 ms of transfers, the same
   as the CPU. Keeping the GPU off by default is right for this machine.
5. **The `lod` simplification levels (1–3) never touch liquids** (`main.rs` `update_world`; they only change heat in
   `thermal.rs`). In liquid-heavy scenes the only relief is lowering the game speed.
6. **Viscosity is 23 % of the step in `--demo`** (10 ms, 2.4 ms per pass), probably lava. `VISCOUS_SWEEPS` = 12.

## Proposal

1. **Load-driven liquid fallback**, using the existing frame-time measure (`frame_ema`) with a state per zone:
   - *Exact*: current MAC + multigrid-PCG solver.
   - *Cheap*: fewer sub-steps (`MAX_SUB` 4 → 2) and fewer solver iterations (`MAX_ITER` 12 → 4). Same physics, less accurate.
   - *Particles*: zones that are mostly spray, falling or fast liquid switch to the existing `Drop` system
     (`liquid.rs`, `fly_drops`). Rules mimicking the liquid: gravity, sliding on slopes, spreading when blocked,
     level piling, mass conserved. Calm bodies keep the solver or sleep.
2. **Quick wins** (no behaviour change): lower or space out viscosity sweeps; spread the `structures` spike over steps;
   skip `forces` on unchanged edges.

Suggested order: cheap tier + viscosity + `structures` first (measurable with the bench), particle fallback after.
