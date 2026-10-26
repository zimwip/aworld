//! Where the time of a step goes. Off by default: switched on, every phase of `World::step` is
//! timed and added up, so a test, a benchmark or the game's overlay can say which system costs
//! what. Off, it costs one test per phase (no clock is read: the web build has none).

/// The phases of a step, in the order they run.
/// After `total`, the parts of the heat pass (already counted in `chaleur`), then the water field.
pub const PHASES: [&str; 30] = [
    "cellules", "structures", "chutes", "corps rigides", "poches", "éjectas", "tassement", "vent",
    "écoulement air", "pression", "poussée", "saisons", "chaleur", "orage", "niveaux liquides", "total",
    "  chaleur/conduction", "  chaleur/vent", "  chaleur/air+réactions", "  chaleur/vers l'espace", "  chaleur/soleil",
    "eau en champ", "liquides physiques",
    "  liquides/classement", "  liquides/forces", "  liquides/viscosité", "  liquides/pression", "  liquides/itérations",
    "  liquides/gouttes", "  liquides/transport",
];
pub const TOTAL: usize = 15;
pub const HEAT_CONDUCTION: usize = 16;
pub const HEAT_WIND: usize = 17;
pub const HEAT_AIR: usize = 18;
pub const HEAT_SPACE: usize = 19;
pub const HEAT_SUN: usize = 20;
/// Not in the order of the step: the liquids' physics runs right after the cells. (Index 21, the
/// water field that is gone, is never timed, so the overlay leaves it out.)
pub const LIQUIDS: usize = 22;
/// The parts of the liquids' physics (already counted in `liquides physiques`), one run per
/// sub-step; `LIQ_ITERATIONS` counts one run per iteration of the pressure solver.
pub const LIQ_CLASSIFY: usize = 23;
pub const LIQ_FORCES: usize = 24;
pub const LIQ_VISCOSITY: usize = 25;
pub const LIQ_PRESSURE: usize = 26;
pub const LIQ_ITERATIONS: usize = 27;
pub const LIQ_DROPS: usize = 28;
pub const LIQ_CARRY: usize = 29;

#[derive(Clone)]
pub struct Profile {
    pub on: bool,
    /// Nanoseconds spent in each phase since the last `reset`.
    pub ns: [u64; PHASES.len()],
    /// Times each phase ran since the last `reset`.
    pub runs: [u32; PHASES.len()],
    /// Steps since the last `reset`.
    pub steps: u32,
}

impl Default for Profile {
    fn default() -> Self {
        Profile { on: false, ns: [0; PHASES.len()], runs: [0; PHASES.len()], steps: 0 }
    }
}

impl Profile {
    pub fn reset(&mut self) {
        let on = self.on;
        *self = Profile { on, ..Default::default() };
    }

    /// Average milliseconds a phase costs per step (spread over every step, also those where
    /// it does not run).
    pub fn ms_per_step(&self, phase: usize) -> f64 {
        self.ns[phase] as f64 / 1e6 / self.steps.max(1) as f64
    }

    /// One line per phase, the dearest first.
    pub fn report(&self) -> String {
        let mut order: Vec<usize> = (0..PHASES.len()).collect();
        order.sort_by(|&a, &b| self.ns[b].cmp(&self.ns[a]));
        let total = self.ns[TOTAL].max(1) as f64;
        let mut out = String::new();
        for i in order {
            if self.runs[i] == 0 {
                continue;
            }
            let per_run = self.ns[i] as f64 / 1e6 / self.runs[i] as f64;
            out += &format!(
                "{:>16}  {:7.3} ms/pas  {:5.1} %  ({} passes, {:.3} ms chacune)\n",
                PHASES[i],
                self.ms_per_step(i),
                100.0 * self.ns[i] as f64 / total,
                self.runs[i],
                per_run
            );
        }
        out
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub type Clock = Option<std::time::Instant>;
#[cfg(target_arch = "wasm32")]
pub type Clock = Option<()>;

impl Profile {
    #[inline]
    pub fn start(&self) -> Clock {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.on.then(std::time::Instant::now)
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }

    #[inline]
    pub fn stop(&mut self, phase: usize, clock: Clock) {
        self.stop_n(phase, clock, 1);
    }

    /// `stop`, counting `n` runs (a loop of sweeps timed as a whole).
    #[inline]
    pub fn stop_n(&mut self, phase: usize, clock: Clock, n: u32) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(t) = clock {
            self.ns[phase] += t.elapsed().as_nanos() as u64;
            self.runs[phase] += n;
        }
        #[cfg(target_arch = "wasm32")]
        let _ = (phase, clock, n);
    }
}
