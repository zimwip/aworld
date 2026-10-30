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
   as the CPU. Keeping the GPU off by default is right for this machine. *(Later, 30 October: after the liquids' work
   on the CPU and the card changed, the same Arc B390 runs `déluge` in 23 ms on the card against 45 on the CPU, but
   `orage` in 26 against 19. `auto`, now the default, measures both sides and keeps the liquids where they cost less.)*
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

## What was done (October 29)

Measured on the cloud machine of the work (4 cores), `AWORLD_BENCH_TIER` setting the liquids' tier:

| Scene | exact | cheap (tier 1) | drops (tier 2) |
|---|---|---|---|
| liquides | 75 ms | 50 ms | 43 ms |
| déluge | 116 ms | 70 ms | 76 ms |
| nuage | 59 ms | 49 ms | 40 ms |
| orage | 30 ms | 25 ms | 21 ms |

- **Quick wins** (the world comes out the same, `bench_fingerprint`): the viscosity sweeps only run over the thick
  edges, on one core when there are few (scene `lave`, the game's demo: 3.4 → 0.9 ms a pass); the structures'
  fingerprints and grouping on every core (−15 to −25 %, `structures_probe`); an edge at rest carries itself in
  `forces`. The structures pass is lighter but not spread over steps: cut between two ticks it would read a stale
  world.
- **Load-driven liquid tiers** (`World::liq_tier`): when the game stays slow and the liquids are the bulk of the work
  (`World::liq_work`, their spans over the sub-steps, above a fifth of the grid), the liquids are simplified before
  the heat. Tier 1: `CHEAP_SUB` = 2 sub-steps a turn, `CHEAP_ITER` = 4 solver iterations. Tier 2: also, in each zone
  where most of the running liquid is free (air under it, falling with air at the end of the liquid under it, or
  spray thrown fast), the free cells go over to drops (`fall_to_drops`), which fly and land on their own; the solver
  keeps the bodies at rest. Mass is conserved (tests). The smooth game brings the heat back first, then the liquids.
  On the flood, tier 2 is no cheaper than tier 1: the landing drops wake the pool they fall in.

## The simple liquid solver (October 30)

A second solver for every liquid (water, oil, lava, the molten metals), switched on by the « Liquides simplifiés » box of
the Monde tab (`World::simple_liquids`), to set beside the detailed one and tune. It keeps the grids, the classes, the
transport (mass exact), the drops and the sleep of the zones, and makes each costly part cheap (`liquid.rs`,
`SIMPLE_*`):

- at most 2 sub-steps a turn (`SIMPLE_SUB`, 4 for the detailed one);
- the velocities carried once (plain semi-Lagrangian, no forward-back correction);
- the pressure starts from a guess from the depth (`hydrostatic`: the weight over each cell under an open surface,
  the gas alone in a column that falls freely, a straight line along a roofed pipe between its open ends) and takes
  one iteration of the multigrid solver (`SIMPLE_ITER`, up to 12 for the detailed one);
- 3 viscosity sweeps instead of 12.

With `SIMPLE_ITER = 0` there is no multigrid at all (`Solver::lines`, then Gauss-Seidel): cheaper again, right for a
U-tube or a dam, but the bench's flood squeezes by a third where it lands.

Side by side (`cargo test --release --bin aworld simple_beside_detailed -- --ignored --nocapture`):

| | detailed | simple |
|---|---|---|
| dam break, front after 1 s | 11.3 m | 11.3 m |
| U-tube, half period | 2.80 s | 2.80 s |
| water, front after 4 s | 192 cells | 169 cells |
| lava, front after 4 s | 58 cells | 76 cells |
| molten metal, front after 4 s | 170 cells | 141 cells |

Bench (`AWORLD_BENCH_SIMPLE=1`), liquids' ms a step on the cloud machine (4 cores):

| Scene | detailed | simple |
|---|---|---|
| liquides | 58 | 30 |
| déluge | 84 | 45 |
| nuage | 40 | 24 |
| orage | 17 | 11 |
| lave | 7.0 | 5.8 |

`flood_probe` takes `AWORLD_BENCH_SIMPLE=1` too, for pictures of the flood on either solver.

## The brush held down (October 30)

Painting brought the images a second down to the steps a second: every frame with a button held borrowed the real world
from the simulation thread, and the thread hands it over only after a whole step (`simthread.rs`). With steps of 30–50 ms
the screen fell to 20–30 images a second as soon as the brush was down.

Now only the click that starts a stroke borrows the world. While the button stays down with a brush tool (matter, eraser
on either button, tunnel), the frame keeps the picture: the stroke is painted onto it at once to be seen, and queued
(`Game::stroke`, `World::Stroke`) for the simulation thread, which paints it into the real world between two steps. The
strokes are numbered; a picture made before a stroke reached the world gets it painted again, so the trail never
flickers. The flamethrower and the shovel still borrow (they touch the trucks and the shovel's load).

`PAINT=1 cargo test --release frame_cost -- --ignored --nocapture` (cloud machine, 4 cores): 52 ms a frame with the old
lend at every frame (`PAINT=lend`), 2 ms with the strokes sent.
