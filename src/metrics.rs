//! The metrics panel (Ctrl+D): where the time of a step goes, part by part of the simulation, so
//! that an optimisation can aim at what costs. Each part of `Game::sim_step` is timed (the world
//! itself in detail, through `world/prof.rs`), gathered over about a second, and shown as
//! milliseconds a step, share of the step and the dearest single step, with the zones that are
//! awake and the last steps drawn as a little graph. Off (panel hidden and no
//! `AWORLD_PROFILE`), it costs one test per part.

use macroquad::prelude::*;

use crate::ui::{rrect, Ui, ACCENT, DIM, GREEN, RED, TEXT};
use crate::world::prof::{self, Profile, PHASES};

/// The parts of a step of the game, in the order they run (`Game::sim_step`).
pub const PARTS: [&str; 11] = [
    "monde", "dynamite", "capteurs", "voitures", "débris", "blocs et ponts", "mécanismes", "tuyaux et pompes",
    "empreintes des blocs", "feu, rouille, explosions", "pompiers",
];
pub const WORLD: usize = 0;
pub const DYNAMITE: usize = 1;
pub const SENSORS: usize = 2;
pub const CARS: usize = 3;
pub const DEBRIS: usize = 4;
pub const BLOCKS: usize = 5;
pub const MECH: usize = 6;
pub const PIPES: usize = 7;
pub const STAMPS: usize = 8;
pub const AFTERMATH: usize = 9;
pub const TRUCKS: usize = 10;
/// Not a part of the step: what the frame does around the steps (animals, the kid, the plane,
/// the watch on slow frames...). Kept apart, it is not counted in the step.
pub const AROUND: usize = 11;
const N: usize = PARTS.len() + 1;

/// How long the figures are gathered before they are shown, in seconds.
const EVERY: f64 = 1.0;
/// Steps kept for the graph.
const HISTORY: usize = 240;
/// The time of one step at x1 (60 steps a second): above it, the world cannot keep up.
const BUDGET_MS: f32 = 1000.0 / 60.0;

/// Seconds on a steady clock. Not macroquad's `get_time`: the steps may run away from the main
/// thread, where it cannot be read.
#[cfg(not(target_arch = "wasm32"))]
pub fn now() -> f64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}
#[cfg(target_arch = "wasm32")]
pub fn now() -> f64 {
    get_time()
}

/// The figures of one gathering.
#[derive(Clone, Default)]
pub struct Window {
    /// Nanoseconds spent in each part, and the dearest single step of each.
    ns: [u64; N],
    worst_ns: [u64; N],
    /// The dearest whole step.
    worst_step_ns: u64,
    steps: u32,
    /// Frames (for what is done once a frame around the steps).
    frames: u32,
    /// The world's own phases (empty on the web: it has no clock for them).
    world: Profile,
    /// How long the gathering lasted, in seconds.
    span: f64,
}

impl Window {
    fn ms_per_step(&self, part: usize) -> f32 {
        self.ns[part] as f32 / 1e6 / self.steps.max(1) as f32
    }

    fn step_ns(&self) -> u64 {
        self.ns[..PARTS.len()].iter().sum()
    }
}

#[derive(Clone)]
pub struct Metrics {
    /// The panel is on the screen (Ctrl+D).
    pub shown: bool,
    /// `AWORLD_PROFILE=1`: a report on the terminal every 600 steps.
    log: bool,
    log_acc: Window,
    /// The gathering under way, the last one finished (what the panel shows), and when the
    /// one under way began.
    acc: Window,
    pub last: Window,
    since: f64,
    /// What each part has cost in the step under way.
    this_step: [u64; N],
    /// The last steps, in milliseconds, oldest first from `head`.
    history: [f32; HISTORY],
    head: usize,
}

impl Metrics {
    pub fn new() -> Self {
        Metrics {
            shown: false,
            log: std::env::var_os("AWORLD_PROFILE").is_some_and(|v| v != "0"),
            log_acc: Window::default(),
            acc: Window::default(),
            last: Window::default(),
            since: now(),
            this_step: [0; N],
            history: [0.0; HISTORY],
            head: 0,
        }
    }

    /// Whether anything is measured.
    pub fn on(&self) -> bool {
        self.shown || self.log
    }

    /// A clock for `lap` (None when nothing is measured: no clock is read).
    #[inline]
    pub fn start(&self) -> Option<f64> {
        self.on().then(now)
    }

    /// The time since `clock` goes to `part`, and the clock starts again for the next part.
    #[inline]
    pub fn lap(&mut self, part: usize, clock: &mut Option<f64>) {
        if let Some(t) = clock {
            let n = now();
            let ns = ((n - *t) * 1e9) as u64;
            self.acc.ns[part] += ns;
            self.this_step[part] += ns;
            *t = n;
        }
    }

    /// A step of the game is over: its parts and the world's phases (taken from `world`, whose
    /// figures start again from nothing) go into the gathering.
    pub fn end_step(&mut self, world: &mut Profile) {
        if !self.on() {
            return;
        }
        let step: u64 = self.this_step[..PARTS.len()].iter().sum();
        for i in 0..N {
            self.acc.worst_ns[i] = self.acc.worst_ns[i].max(self.this_step[i]);
        }
        self.acc.worst_step_ns = self.acc.worst_step_ns.max(step);
        self.acc.steps += 1;
        if self.log {
            let one = self.acc_last_step(step);
            add(&mut self.log_acc, &one);
            self.log_acc.world.add(world);
        }
        self.acc.world.add(world);
        world.reset();
        self.this_step = [0; N];
        self.history[self.head] = step as f32 / 1e6;
        self.head = (self.head + 1) % HISTORY;
        self.roll();
    }

    /// What the frame does around the steps: counted apart. `first`: the first call of a frame.
    pub fn around(&mut self, clock: Option<f64>, first: bool) {
        let mut clock = clock;
        if first && clock.is_some() {
            self.acc.frames += 1;
        }
        self.lap(AROUND, &mut clock);
        self.acc.worst_ns[AROUND] = self.acc.worst_ns[AROUND].max(self.this_step[AROUND]);
        self.this_step[AROUND] = 0;
        self.roll();
    }

    /// The step just ended, alone, as a gathering (for the terminal's report).
    fn acc_last_step(&self, step: u64) -> Window {
        let mut w = Window { steps: 1, worst_step_ns: step, ..Default::default() };
        w.ns = self.this_step;
        w.worst_ns = self.this_step;
        w
    }

    /// A gathering that has lasted long enough is shown.
    fn roll(&mut self) {
        let t = now();
        if t - self.since >= EVERY {
            self.acc.span = t - self.since;
            self.last = std::mem::take(&mut self.acc);
            self.since = t;
        }
    }

    /// `AWORLD_PROFILE=1`: every 600 steps, the report to print (the parts of the game, then the
    /// world's phases), and the count starts again.
    pub fn take_log(&mut self) -> Option<String> {
        if !self.log {
            return None;
        }
        let out = (self.log_acc.steps >= 600).then(|| {
            let w = &self.log_acc;
            let total = w.step_ns().max(1) as f64;
            let mut out = String::new();
            for i in order(w) {
                out += &format!("{:>26}  {:7.3} ms/pas  {:5.1} %  (pire {:.2} ms)\n", PARTS[i], w.ms_per_step(i), 100.0 * w.ns[i] as f64 / total, w.worst_ns[i] as f64 / 1e6);
            }
            out + "  dont le monde :\n" + &w.world.report()
        });
        if out.is_some() {
            self.log_acc = Window::default();
        }
        out
    }

    /// The panel, above the frame counter in the bottom right corner.
    pub fn draw(&self, ui: &Ui, sw: f32, sh: f32, info: &Info) {
        if !self.shown {
            return;
        }
        let w = &self.last;
        let rows = rows(w);
        let line = 17.0;
        let (pw, ph) = (420.0, 172.0 + line * rows.len() as f32);
        let r = Rect::new(sw - pw - 8.0, (sh - 38.0 - ph).max(8.0), pw, ph);
        rrect(r, 10.0, Color::new(0.05, 0.06, 0.1, 0.86));
        let (x, mut y) = (r.x + 12.0, r.y + 22.0);
        ui.text("Métriques de la simulation", x, y, 16, TEXT);
        let hint = "Ctrl+D";
        ui.text(hint, r.x + pw - 12.0 - ui.text_w(hint, 13), y, 13, DIM);
        y += 20.0;

        let step_ms = w.step_ns() as f32 / 1e6 / w.steps.max(1) as f32;
        let busy = if w.span > 0.0 { w.step_ns() as f64 / 1e9 / w.span } else { 0.0 };
        if w.steps == 0 {
            ui.text("monde en pause : rien à mesurer", x, y, 14, DIM);
        } else {
            let colour = if step_ms <= BUDGET_MS * 0.5 { GREEN } else if step_ms <= BUDGET_MS { ACCENT } else { RED };
            let head = format!("{step_ms:.2} ms/pas");
            ui.text(&head, x, y, 15, colour);
            ui.text(
                &format!("pire {:.1} ms · {} pas en {:.1} s · {:.0} % du temps", w.worst_step_ns as f32 / 1e6, w.steps, w.span, busy * 100.0),
                x + ui.text_w(&head, 15) + 8.0,
                y,
                13,
                DIM,
            );
        }
        y += 18.0;
        ui.text(
            &format!(
                "zones {s}×{s} éveillées {}/{} ({:.0} %) · simplification {}/3 · x{}",
                info.awake,
                info.zones,
                100.0 * info.awake as f32 / info.zones.max(1) as f32,
                info.lod,
                info.speed,
                s = crate::world::sleep::CS
            ),
            x,
            y,
            13,
            DIM,
        );
        y += 16.0;
        ui.text(&format!("{} blocs · {} poutres · {} voitures · {} tuyaux", info.blocks, info.members, info.cars, info.pipes), x, y, 13, DIM);
        y += 8.0;

        // The last steps: a bar each, the line of the budget of a step at x1.
        let g = Rect::new(x, y, pw - 24.0, 44.0);
        draw_rectangle(g.x, g.y, g.w, g.h, Color::new(1.0, 1.0, 1.0, 0.04));
        let top = self.history.iter().fold(BUDGET_MS, |a, &b| a.max(b));
        let bw = g.w / HISTORY as f32;
        for k in 0..HISTORY {
            let ms = self.history[(self.head + k) % HISTORY];
            let h = g.h * ms / top;
            let c = if ms <= BUDGET_MS * 0.5 { GREEN } else if ms <= BUDGET_MS { ACCENT } else { RED };
            draw_rectangle(g.x + k as f32 * bw, g.y + g.h - h, bw.max(1.0), h, Color::new(c.r, c.g, c.b, 0.75));
        }
        let by = g.y + g.h - g.h * BUDGET_MS / top;
        draw_line(g.x, by, g.x + g.w, by, 1.0, Color::new(1.0, 1.0, 1.0, 0.35));
        ui.text(&format!("{top:.0} ms"), g.x + 3.0, g.y + 11.0, 11, DIM);
        y = g.y + g.h + 18.0;

        let (cms, cpct, cworst) = (x + 210.0, x + 268.0, x + 320.0);
        ui.text("partie", x, y, 12, DIM);
        ui.text("ms/pas", cms, y, 12, DIM);
        ui.text("%", cpct + 12.0, y, 12, DIM);
        ui.text("pire", cworst, y, 12, DIM);
        y += 4.0;
        let total = w.step_ns().max(1) as f32;
        for row in &rows {
            y += line;
            let share = row.ns as f32 / total;
            // The share as a bar behind the line.
            if row.depth < 2 {
                draw_rectangle(x - 4.0, y - 12.0, (cms - x - 8.0) * share.min(1.0), 15.0, Color::new(0.3, 0.55, 1.0, if row.depth == 0 { 0.28 } else { 0.16 }));
            }
            let col = if row.depth == 0 { TEXT } else { DIM };
            let size = if row.depth == 0 { 14 } else { 13 };
            ui.text(&row.name, x + 12.0 * row.depth as f32, y, size, col);
            let per_step = row.ns as f32 / 1e6 / w.steps.max(1) as f32;
            ui.text(&format!("{per_step:6.3}"), cms, y, size, col);
            ui.text(&format!("{:3.0}", share * 100.0), cpct, y, size, col);
            let worst = row.worst_ns as f32 / 1e6;
            let wc = if worst > BUDGET_MS { RED } else { col };
            ui.text(&format!("{worst:5.2}"), cworst, y, size, wc);
        }
        y += line + 2.0;
        let around = w.ns[AROUND] as f32 / 1e6 / w.frames.max(1) as f32;
        ui.text(
            &format!("par image : dessin du monde {:.2} · interface {:.2} · hors pas {around:.2} ms", info.scene_ms, info.ui_ms),
            x,
            y,
            12,
            DIM,
        );
    }
}

fn add(to: &mut Window, w: &Window) {
    for i in 0..N {
        to.ns[i] += w.ns[i];
        to.worst_ns[i] = to.worst_ns[i].max(w.worst_ns[i]);
    }
    to.worst_step_ns = to.worst_step_ns.max(w.worst_step_ns);
    to.steps += w.steps;
}

/// The parts of the step, the dearest first.
fn order(w: &Window) -> Vec<usize> {
    let mut o: Vec<usize> = (0..PARTS.len()).collect();
    o.sort_by(|&a, &b| w.ns[b].cmp(&w.ns[a]));
    o
}

/// A line of the table: a part of the step, a phase of the world under "monde", or a part of a
/// phase (the heat's, the liquids') under that phase.
struct Row {
    name: String,
    depth: u8,
    ns: u64,
    worst_ns: u64,
}

/// Below this share of the step a line is left out (it would only lengthen the table).
const SMALL: f64 = 0.004;

fn rows(w: &Window) -> Vec<Row> {
    let total = w.step_ns().max(1) as f64;
    let small = |ns: u64| (ns as f64) < total * SMALL;
    let mut out = vec![];
    for i in order(w) {
        if w.ns[i] == 0 || small(w.ns[i]) {
            continue;
        }
        out.push(Row { name: PARTS[i].to_string(), depth: 0, ns: w.ns[i], worst_ns: w.worst_ns[i] });
        if i != WORLD {
            continue;
        }
        // The world's phases (its sub-parts start with two spaces: "  chaleur/conduction").
        let p = &w.world;
        let mut top: Vec<usize> = (0..PHASES.len()).filter(|&k| k != prof::TOTAL && !PHASES[k].starts_with("  ")).collect();
        top.sort_by(|&a, &b| p.ns[b].cmp(&p.ns[a]));
        for k in top {
            if p.runs[k] == 0 || small(p.ns[k]) {
                continue;
            }
            out.push(Row { name: PHASES[k].to_string(), depth: 1, ns: p.ns[k], worst_ns: p.worst_ns[k] });
            // "chaleur" takes the "  chaleur/..." parts, "liquides physiques" the "  liquides/...".
            let word = PHASES[k].split(' ').next().unwrap_or("");
            let mut subs: Vec<usize> = (0..PHASES.len())
                .filter(|&s| PHASES[s].trim_start().split('/').next() == Some(word) && PHASES[s].starts_with("  "))
                .collect();
            subs.sort_by(|&a, &b| p.ns[b].cmp(&p.ns[a]));
            for s in subs {
                if p.runs[s] == 0 || small(p.ns[s]) {
                    continue;
                }
                let name = PHASES[s].trim_start().split_once('/').map_or(PHASES[s], |(_, n)| n);
                out.push(Row { name: name.to_string(), depth: 2, ns: p.ns[s], worst_ns: p.worst_ns[s] });
            }
        }
    }
    out
}

/// What the panel shows beside the times, read from the game when it is drawn.
pub struct Info {
    pub awake: usize,
    pub zones: usize,
    pub lod: u8,
    pub speed: usize,
    pub blocks: usize,
    pub members: usize,
    pub cars: usize,
    pub pipes: usize,
    /// The last frame's drawing, in milliseconds: the world and the interface.
    pub scene_ms: f32,
    pub ui_ms: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_is_split_into_its_parts() {
        let mut m = Metrics::new();
        m.shown = true;
        let mut world = Profile::default();
        world.on = true;
        for _ in 0..3 {
            let mut clock = m.start();
            std::thread::sleep(std::time::Duration::from_millis(2));
            m.lap(WORLD, &mut clock);
            std::thread::sleep(std::time::Duration::from_millis(1));
            m.lap(CARS, &mut clock);
            m.end_step(&mut world);
        }
        let w = &m.acc;
        assert_eq!(w.steps, 3);
        assert!(w.ns[WORLD] > w.ns[CARS], "the world slept longer than the cars");
        assert!(w.worst_step_ns >= 3_000_000);
        assert!(w.worst_ns[WORLD] * 3 >= w.ns[WORLD], "the dearest step is at least the average");
        let names: Vec<String> = rows(w).into_iter().map(|r| r.name).collect();
        assert_eq!(names[..2], ["monde".to_string(), "voitures".to_string()]);
    }

    #[test]
    fn hidden_it_reads_no_clock() {
        let mut m = Metrics::new();
        m.log = false;
        let mut clock = m.start();
        assert!(clock.is_none());
        m.lap(WORLD, &mut clock);
        m.end_step(&mut Profile::default());
        assert_eq!(m.acc.steps, 0);
    }

    #[test]
    fn the_worlds_phases_come_under_the_world() {
        let mut w = Window { steps: 1, ..Default::default() };
        w.ns[WORLD] = 10_000_000;
        let heat = PHASES.iter().position(|&p| p == "chaleur").unwrap();
        w.world.ns[heat] = 6_000_000;
        w.world.runs[heat] = 1;
        w.world.ns[prof::HEAT_SUN] = 4_000_000;
        w.world.runs[prof::HEAT_SUN] = 1;
        let r = rows(&w);
        let names: Vec<(&str, u8)> = r.iter().map(|r| (r.name.as_str(), r.depth)).collect();
        assert_eq!(names, [("monde", 0), ("chaleur", 1), ("soleil", 2)]);
    }
}
