//! The fluids tab: pipes and valves, how they are drawn, driven and run. The hydraulics itself is in
//! `pipes.rs`.

use super::*;
use crate::mechgame::snap_to_axes;
use crate::pipes::{Driver, Event as PipeEvent, Pump, Valve};

const PIPE_MIN: f32 = 8.0;
const PIPE_MAX: f32 = 300.0;
const PIPE_R: f32 = 2.2;
const IRON_OR_STEEL: &[Mat] = &[Mat::Iron, Mat::Steel];

fn v2f(p: (f32, f32)) -> Vec2 {
    vec2(p.0, p.1)
}

fn tup(v: Vec2) -> (f32, f32) {
    (v.x, v.y)
}

impl Game {
    /// Where a pipe end clicked at `at` lands: on the end of another pipe, or in the side of one.
    pub fn pipe_snap(&self, at: Vec2) -> Vec2 {
        let pl = &self.mech.plumbing;
        let mut best: Option<(f32, Vec2)> = None;
        for p in &pl.pipes {
            for q in [p.a, p.b] {
                let d = (v2f(q) - at).length();
                if d < 6.0 && best.map_or(true, |b| d < b.0) {
                    best = Some((d, v2f(q)));
                }
            }
        }
        if let Some((_, q)) = best {
            return q;
        }
        let mut side: Option<(f32, Vec2)> = None;
        for p in &pl.pipes {
            let (d, t) = p.distance_to(tup(at));
            if d < 4.0 && t > 0.02 && t < 0.98 && side.map_or(true, |s| d < s.0) {
                side = Some((d, v2f(p.point(t))));
            }
        }
        side.map_or(at, |s| s.1)
    }

    /// The second end of a pipe from `from`: straightened to a horizontal, vertical or diagonal run
    /// and rounded to whole fives, unless it clips onto another pipe.
    pub fn pipe_end(&self, from: Vec2, at: Vec2) -> Vec2 {
        let snapped = self.pipe_snap(at);
        if (snapped - at).length() > 0.01 {
            return snapped;
        }
        let d = at - from;
        let len = d.length();
        if len < 1.0 {
            return at;
        }
        let dir = snap_to_axes(d / len, 6.0);
        from + dir * ((len / 5.0).round().max(1.0) * 5.0).min(PIPE_MAX)
    }

    pub fn valve_world(&self, v: &Valve) -> Option<Vec2> {
        let p = self.mech.plumbing.pipes.iter().find(|p| p.uid == v.pipe)?;
        Some(v2f(p.point(v.t)))
    }

    pub fn valve_at(&self, at: Vec2) -> Option<usize> {
        self.mech
            .plumbing
            .valves
            .iter()
            .enumerate()
            .filter_map(|(i, v)| self.valve_world(v).map(|p| (i, (p - at).length())))
            .filter(|(_, d)| *d < 7.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    pub fn pump_world(&self, p: &Pump) -> Option<Vec2> {
        let pipe = self.mech.plumbing.pipes.iter().find(|q| q.uid == p.pipe)?;
        Some(v2f(pipe.point(p.t)))
    }

    pub fn pump_at(&self, at: Vec2) -> Option<usize> {
        self.mech
            .plumbing
            .pumps
            .iter()
            .enumerate()
            .filter_map(|(i, p)| self.pump_world(p).map(|q| (i, (q - at).length())))
            .filter(|(_, d)| *d < 8.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    fn pipe_at(&self, at: Vec2) -> Option<(usize, f32)> {
        self.mech
            .plumbing
            .pipes
            .iter()
            .enumerate()
            .map(|(i, p)| (i, p.distance_to(tup(at))))
            .filter(|(_, (d, _))| *d < PIPE_R + 2.5)
            .min_by(|a, b| (a.1).0.total_cmp(&(b.1).0))
            .map(|(i, (_, t))| (i, t))
    }

    fn fluid_pay(&mut self, units: u32, near: Vec2) -> bool {
        if self.free_now() {
            return true;
        }
        self.census = self.world.census();
        let have = self.stock(IRON_OR_STEEL);
        if have < units {
            self.say(format!("Pas assez de fer ou d'acier : il en faut {units}, il y en a {have}"));
            return false;
        }
        self.world.consume(IRON_OR_STEEL, units, (near.x as i32, near.y as i32));
        true
    }

    /// What the selected fluid tool costs.
    pub fn fluid_cost(&self, at: Vec2) -> Option<(&'static [Mat], u32)> {
        let units = match self.tool {
            Tool::Pipe => match self.pipe_from {
                Some(from) => ((self.pipe_end(from, at) - from).length().min(PIPE_MAX) * 0.12).ceil() as u32,
                None => 6,
            },
            Tool::Valve => 8,
            Tool::Pump => 20,
            _ => return None,
        };
        Some((IRON_OR_STEEL, units))
    }

    pub fn fluid_click(&mut self, at: Vec2) {
        match self.tool {
            Tool::Pipe => self.pipe_click(at),
            Tool::Valve => self.valve_click(at),
            Tool::Pump => self.pump_click(at),
            _ => {}
        }
    }

    fn pipe_click(&mut self, at: Vec2) {
        let Some(from) = self.pipe_from else {
            self.pipe_from = Some(self.pipe_snap(at));
            return;
        };
        let to = self.pipe_end(from, at);
        let len = (to - from).length();
        if len < PIPE_MIN {
            self.say("Le tuyau est trop court : éloigne le deuxième clic");
            return;
        }
        if !self.fluid_pay((len * 0.12).ceil() as u32, to) {
            return;
        }
        self.checkpoint();
        let uid = mech::next_uid();
        self.mech.plumbing.add_pipe(tup(from), tup(to), uid, mech::next_uid);
        // The next pipe starts where this one ends, so that a run of clicks draws a pipework.
        self.pipe_from = Some(to);
    }

    /// A click on a pipe sets a pump there (pushing from the pipe's first end to its second); a click
    /// on a pump turns it round, then off, then on again.
    fn pump_click(&mut self, at: Vec2) {
        if let Some(i) = self.pump_at(at) {
            let p = &mut self.mech.plumbing.pumps[i];
            if p.driver.is_some() {
                self.say("Cette pompe est menée par un engrenage : sa poussée suit sa vitesse");
            } else if p.on && p.head > 0.0 {
                p.head = -p.head;
                self.say("Pompe : sens inverse");
            } else if p.on {
                p.on = false;
                self.say("Pompe arrêtée");
            } else {
                p.on = true;
                p.head = p.head.abs();
                self.say("Pompe en marche");
            }
            return;
        }
        let Some((pi, t)) = self.pipe_at(at) else {
            self.say("Clique sur un tuyau pour y poser une pompe");
            return;
        };
        if !self.fluid_pay(20, at) {
            return;
        }
        self.checkpoint();
        let pipe = self.mech.plumbing.pipes[pi].uid;
        self.mech.plumbing.pumps.push(Pump::new(mech::next_uid(), pipe, t.clamp(0.08, 0.92)));
        self.say("Pompe posée : elle pousse du premier bout du tuyau vers le second (clique-la pour l'inverser ou l'arrêter, ou relie-la à un engrenage avec l'outil Axe)");
    }

    /// The Axe tool reaching a pump: a gear turns it, and the lift follows the gear's speed.
    pub fn link_pump(&mut self, pump: u32, gear: u32) {
        if let Some(p) = self.mech.plumbing.pumps.iter_mut().find(|p| p.uid == pump) {
            p.driver = Some(gear);
            self.say("Pompe reliée à l'engrenage : plus il tourne vite, plus elle pousse (et elle freine le moteur)");
        }
    }

    fn valve_click(&mut self, at: Vec2) {
        if let Some(i) = self.valve_at(at) {
            let v = &mut self.mech.plumbing.valves[i];
            if v.driver.is_some() {
                self.say("Cette vanne est menée par un engrenage ou un rail");
            } else {
                v.open = if v.open > 0.5 { 0.0 } else { 1.0 };
                let shut = v.open < 0.5;
                self.say(if shut { "Vanne fermée" } else { "Vanne ouverte" });
            }
            return;
        }
        let Some((pi, t)) = self.pipe_at(at) else {
            self.say("Clique sur un tuyau pour y poser une vanne (au bout, c'est un robinet)");
            return;
        };
        if !self.fluid_pay(8, at) {
            return;
        }
        self.checkpoint();
        let pipe = self.mech.plumbing.pipes[pi].uid;
        let mut v = Valve::new(mech::next_uid(), pipe, t.clamp(0.04, 0.96));
        v.turns = self.valve_turns;
        self.mech.plumbing.valves.push(v);
        self.say("Vanne posée : clique-la pour l'ouvrir ou la fermer, ou relie-la à un engrenage ou un rail avec l'outil Axe");
    }

    /// Removes the pipe, pump or valve under `at`. Returns whether there was one.
    pub fn fluid_erase(&mut self, at: Vec2) -> bool {
        if let Some(i) = self.pump_at(at) {
            self.checkpoint();
            self.mech.plumbing.pumps.remove(i);
            return true;
        }
        if let Some(i) = self.valve_at(at) {
            self.checkpoint();
            self.mech.plumbing.valves.remove(i);
            return true;
        }
        if let Some((i, _)) = self.pipe_at(at) {
            self.checkpoint();
            let uid = self.mech.plumbing.pipes[i].uid;
            self.mech.plumbing.remove_pipe(uid);
            return true;
        }
        false
    }

    /// The Axe tool reaching a valve: a gear or a rail will turn it.
    pub fn link_valve(&mut self, valve: u32, driver: Driver) {
        let turns = self.valve_turns;
        let Some(v) = self.mech.plumbing.valves.iter_mut().find(|v| v.uid == valve) else { return };
        v.driver = Some(driver);
        v.turns = turns;
        self.say(match driver {
            Driver::Gear(_) => "Vanne reliée à l'engrenage : elle s'ouvre ou se ferme en quelques tours, puis le moteur cale (ou repart dans l'autre sens)",
            Driver::Slider(_) => "Vanne reliée au rail : elle s'ouvre quand le chariot avance",
        });
    }

    // ----- simulation -------------------------------------------------------

    /// Before the simulation runs: a gear that turns a valve may only turn through its travel.
    pub fn fluid_prepare(&mut self) {
        let valves: Vec<(u32, f32, f32, Option<Driver>)> = self.mech.plumbing.valves.iter().map(|v| (v.uid, v.open, v.turns, v.driver)).collect();
        for (_, open, turns, driver) in valves {
            if let Some(Driver::Gear(g)) = driver {
                if let Some(gi) = self.mech.gear_index(g) {
                    let span = turns.max(0.25) * std::f32::consts::TAU;
                    let th = self.mech.gears[gi].angle;
                    self.mech.gears[gi].limit = Some((th - open * span, th + (1.0 - open) * span));
                }
            }
        }
    }

    /// A pump driven by a gear is a load on it: the harder it lifts the more it holds the gear back.
    pub fn pump_loads(&mut self) {
        for i in 0..self.mech.plumbing.pumps.len() {
            let p = &self.mech.plumbing.pumps[i];
            let Some(g) = p.driver else { continue };
            let flow = self.mech.plumbing.pipes.iter().find(|q| q.uid == p.pipe).map_or(0.0, |q| q.flow.abs());
            let load = (p.head.abs() * 0.02 + flow * p.head.abs() * 0.04).min(16.0);
            if let Some(gi) = self.mech.gear_index(g) {
                let dir = self.mech.gears[gi].omega.signum();
                self.mech.gears[gi].ext_torque -= dir * load;
            }
        }
    }

    pub fn fluid_step(&mut self) {
        if self.mech.plumbing.is_empty() {
            return;
        }
        for i in 0..self.mech.plumbing.valves.len() {
            let (driver, turns) = (self.mech.plumbing.valves[i].driver, self.mech.plumbing.valves[i].turns);
            let open = match driver {
                Some(Driver::Gear(g)) => self.mech.gear_index(g).and_then(|gi| {
                    let gear = &self.mech.gears[gi];
                    gear.limit.map(|(lo, _)| ((gear.angle - lo) / (turns.max(0.25) * std::f32::consts::TAU)).clamp(0.0, 1.0))
                }),
                Some(Driver::Slider(s)) => self.mech.slider_index(s).map(|si| {
                    let sl = &self.mech.sliders[si];
                    let (lo, hi) = (sl.margin, (sl.len - sl.margin).max(sl.margin + 1.0));
                    ((sl.s - lo) / (hi - lo)).clamp(0.0, 1.0)
                }),
                None => None,
            };
            if let Some(o) = open {
                self.mech.plumbing.valves[i].open = o;
            }
        }
        for i in 0..self.mech.plumbing.pumps.len() {
            if let Some(g) = self.mech.plumbing.pumps[i].driver {
                if let Some(gi) = self.mech.gear_index(g) {
                    let w = self.mech.gears[gi].omega;
                    self.mech.plumbing.pumps[i].head = (w * 1300.0).clamp(-220.0, 220.0);
                    self.mech.plumbing.pumps[i].on = w.abs() > 0.004;
                }
            }
        }
        let events = self.mech.plumbing.step(&mut self.world);
        for e in events {
            match e {
                PipeEvent::SiphonBroke(_) => self.say("Le siphon s'est désamorcé : sa bosse est trop haute pour la pression de l'air"),
            }
        }
    }

    // ----- panel ------------------------------------------------------------

    pub fn panel_fluid(&mut self, ui: &mut Ui, top: f32) {
        let free = self.free_now();
        let tools: [(Tool, &str, &str); 3] = [
            (Tool::Pipe, "Tuyau", "Deux clics : un tuyau droit (il se redresse en horizontal, vertical ou diagonale et s'accroche aux autres). Les clics suivants continuent la tuyauterie ; un tuyau qui finit sur le flanc d'un autre fait un branchement. Chaque bout libre est ouvert : il aspire le liquide où il plonge, ou le rejette. Clic droit pour finir."),
            (Tool::Pump, "Pompe", "Clique sur un tuyau pour y poser une pompe : elle pousse le liquide du premier bout du tuyau vers le second et peut le monter plus haut qu'un siphon ne le ferait. Clique-la pour l'inverser ou l'arrêter. Avec l'outil Axe, relie-la à un engrenage : la poussée suit sa vitesse et elle le freine."),
            (Tool::Valve, "Vanne", "Clique sur un tuyau pour poser une vanne (au bout d'un tuyau, c'est un robinet). Clique-la pour l'ouvrir ou la fermer. Avec l'outil Axe, relie-la à un engrenage (elle s'ouvre en quelques tours puis bute) ou à un rail."),
        ];
        for (i, (tool, label, text)) in tools.iter().enumerate() {
            let r = Self::tile_rect(i, 3, top, 54.0);
            if ui.hover(r) {
                self.hover_info = Some((label.to_string(), format!("{text} Coût : 0,12 par cellule de tuyau, 8 par vanne (fer ou acier).")));
            }
            let short = !free && matches!(tool, Tool::Valve | Tool::Pump) && self.stock(IRON_OR_STEEL) < if matches!(tool, Tool::Pump) { 20 } else { 8 };
            let tool = *tool;
            if ui.tile(r, label, self.tool == tool, |s| {
                let c = vec2(s.x + s.w / 2.0, s.y + s.h / 2.0);
                match tool {
                    Tool::Pipe => {
                        draw_line(c.x - 18.0, c.y + 6.0, c.x + 4.0, c.y + 6.0, 7.0, Color::from_rgba(120, 128, 150, 255));
                        draw_line(c.x + 4.0, c.y + 6.0, c.x + 4.0, c.y - 8.0, 7.0, Color::from_rgba(120, 128, 150, 255));
                        draw_line(c.x - 18.0, c.y + 6.0, c.x + 4.0, c.y + 6.0, 3.0, Color::from_rgba(70, 140, 230, 255));
                        draw_line(c.x + 4.0, c.y + 6.0, c.x + 4.0, c.y - 8.0, 3.0, Color::from_rgba(70, 140, 230, 255));
                    }
                    Tool::Pump => {
                        draw_line(c.x - 18.0, c.y, c.x + 18.0, c.y, 7.0, Color::from_rgba(120, 128, 150, 255));
                        draw_circle(c.x, c.y, 11.0, Color::from_rgba(70, 74, 92, 255));
                        draw_circle_lines(c.x, c.y, 11.0, 2.0, Color::from_rgba(70, 150, 255, 255));
                        draw_triangle(vec2(c.x - 5.0, c.y - 6.0), vec2(c.x - 5.0, c.y + 6.0), vec2(c.x + 7.0, c.y), WHITE);
                    }
                    _ => {
                        draw_line(c.x - 18.0, c.y, c.x + 18.0, c.y, 7.0, Color::from_rgba(120, 128, 150, 255));
                        draw_rectangle(c.x - 5.0, c.y - 9.0, 10.0, 18.0, Color::from_rgba(214, 160, 70, 255));
                        draw_line(c.x, c.y - 9.0, c.x, c.y - 14.0, 2.0, WHITE);
                        draw_line(c.x - 5.0, c.y - 14.0, c.x + 5.0, c.y - 14.0, 3.0, WHITE);
                    }
                }
            }) {
                self.tool = tool;
            }
            if short {
                Self::mark_missing(r);
            }
        }
        let y = top + 66.0;
        ui.text("Tours de vanne (engrenage)", 10.0, y + 11.0, 13, DIM);
        ui.slider(Rect::new(10.0, y + 18.0, PANEL_W - 70.0, 18.0), &mut self.valve_turns, 0.5, 8.0, ACCENT);
        ui.text(&format!("{:.1}", self.valve_turns), PANEL_W - 52.0, y + 31.0, 14, TEXT);
        ui.wrapped(
            "Le liquide va du niveau haut au niveau bas : vases communicants, siphon (la bosse ne peut pas dépasser ce que l'air soutient), écluse (deux bassins et une vanne). Une vanne menée par un engrenage bute après son nombre de tours.",
            10.0,
            y + 58.0,
            PANEL_W - 20.0,
            12,
            DIM,
        );
    }

    // ----- drawing ----------------------------------------------------------

    pub fn draw_pipes(&self, view: View, ui: &Ui, inside: bool) {
        let pl = &self.mech.plumbing;
        let pressure = self.mode == ViewMode::Pressure;
        for p in &pl.pipes {
            let (a, b) = (view.to_screen(v2f(p.a)), view.to_screen(v2f(p.b)));
            let w = PIPE_R * 2.0 * view.s;
            draw_line(a.x, a.y, b.x, b.y, w + 2.5, Color::from_rgba(52, 58, 74, 255));
            draw_line(a.x, a.y, b.x, b.y, w, Color::from_rgba(112, 120, 142, 255));
            // The liquid inside: its colour, or the pressure it is under in the pressure view.
            if p.primed || pressure {
                let (ca, cb) = (self.head_colour(p.head.0, p, 0), self.head_colour(p.head.1, p, 1));
                let n = 12;
                for k in 0..n {
                    let (t0, t1) = (k as f32 / n as f32, (k + 1) as f32 / n as f32);
                    let (q0, q1) = (a + (b - a) * t0, a + (b - a) * t1);
                    let col = if pressure { lerp_col(ca, cb, (t0 + t1) / 2.0) } else { fluid_colour(p.fluid) };
                    if p.primed || pressure {
                        draw_line(q0.x, q0.y, q1.x, q1.y, w * 0.62, if p.primed { col } else { Color::new(col.r, col.g, col.b, 0.3) });
                    }
                }
            }
            // Dashes running along it show which way, and how fast, it flows.
            if p.flow.abs() > 0.02 {
                let d = (b - a).normalize_or_zero();
                let len = (b - a).length();
                let spacing = 11.0 * view.s.max(0.8);
                let phase = (self.time * p.flow.signum() * (60.0 + p.flow.abs() * 90.0) * view.s.max(0.8)).rem_euclid(spacing);
                let mut t = phase;
                while t < len {
                    let c = a + d * t;
                    draw_circle(c.x, c.y, (0.9 + p.flow.abs().min(2.0) * 0.4) * view.s.max(0.8), Color::new(1.0, 1.0, 1.0, 0.8));
                    t += spacing;
                }
            }
        }
        // Open ends: blue when they dip into liquid, orange in the air, with the way it flows.
        for p in &pl.pipes {
            for (e, q) in [(0u8, p.a), (1u8, p.b)] {
                let free = pl.pipes.iter().flat_map(|o| [o.a, o.b]).filter(|o| ((o.0 - q.0).powi(2) + (o.1 - q.1).powi(2)).sqrt() < 3.0).count() == 1;
                if !free {
                    continue;
                }
                let s = view.to_screen(v2f(q));
                let wet = self.world.at(q.0 as i32, q.1 as i32).kind() == world::Kind::Liquid;
                let col = if wet { Color::from_rgba(70, 150, 255, 255) } else { Color::from_rgba(255, 170, 60, 255) };
                draw_circle_lines(s.x, s.y, (PIPE_R + 1.6) * view.s, 2.0, col);
                let into = if e == 0 { p.flow } else { -p.flow };
                if into.abs() > 0.05 {
                    let other = if e == 0 { v2f(p.b) } else { v2f(p.a) };
                    let dir = (view.to_screen(other) - s).normalize_or_zero() * if into > 0.0 { 1.0 } else { -1.0 };
                    let tip = s + dir * 14.0 * view.s.max(0.8);
                    draw_line(s.x, s.y, tip.x, tip.y, 2.0, col);
                }
            }
        }
        for v in &pl.valves {
            let Some(p) = self.valve_world(v) else { continue };
            let pipe = pl.pipes.iter().find(|q| q.uid == v.pipe).unwrap();
            let s = view.to_screen(p);
            let d = (view.to_screen(v2f(pipe.b)) - view.to_screen(v2f(pipe.a))).normalize_or_zero();
            let side = vec2(-d.y, d.x);
            let half = (PIPE_R + 3.0) * view.s;
            // The body, and a gate that slides across the pipe as the valve closes.
            draw_rectangle_ex(s.x, s.y, 7.0 * view.s, half * 2.0, DrawRectangleParams { offset: vec2(0.5, 0.5), rotation: d.y.atan2(d.x), color: Color::from_rgba(70, 74, 92, 255) });
            let gate = side * half * 1.9 * (v.open - 0.5) * -1.0;
            let g = s + gate;
            draw_circle(g.x, g.y, 2.6 * view.s, Color::from_rgba(214, 160, 70, 255));
            let top = s - side * (half + 5.0 * view.s);
            draw_line(s.x - side.x * half, s.y - side.y * half, top.x, top.y, 2.0, WHITE);
            let wheel_col = if v.driver.is_some() { GREEN } else { WHITE };
            draw_circle_lines(top.x, top.y, 3.6 * view.s, 2.0, wheel_col);
            if v.open < 0.02 {
                draw_line(s.x - side.x * half, s.y - side.y * half, s.x + side.x * half, s.y + side.y * half, 3.0, RED);
            }
        }
        for pm in &pl.pumps {
            let Some(p) = self.pump_world(pm) else { continue };
            let pipe = pl.pipes.iter().find(|q| q.uid == pm.pipe).unwrap();
            let s = view.to_screen(p);
            let mut d = (view.to_screen(v2f(pipe.b)) - view.to_screen(v2f(pipe.a))).normalize_or_zero();
            if pm.head < 0.0 {
                d = -d;
            }
            let side = vec2(-d.y, d.x);
            let r = (PIPE_R + 4.5) * view.s;
            draw_circle(s.x, s.y, r, Color::from_rgba(52, 58, 76, 255));
            draw_circle_lines(s.x, s.y, r, 2.0, if pm.on { Color::from_rgba(70, 150, 255, 255) } else { Color::from_rgba(150, 154, 170, 255) });
            let col = if pm.on { WHITE } else { Color::new(1.0, 1.0, 1.0, 0.4) };
            draw_triangle(s - side * r * 0.5 - d * r * 0.45, s + side * r * 0.5 - d * r * 0.45, s + d * r * 0.6, col);
            if pm.driver.is_some() {
                draw_circle(s.x + r * 0.9, s.y - r * 0.9, 2.2 * view.s.max(0.9), GREEN);
            }
        }
        if !inside {
            return;
        }
        let at = view.to_world(ui.mouse);
        match self.tool {
            Tool::Pump => {
                if let Some(i) = self.pump_at(at) {
                    if let Some(p) = self.pump_world(&self.mech.plumbing.pumps[i]) {
                        let s = view.to_screen(p);
                        draw_circle_lines(s.x, s.y, 11.0 * view.s, 2.0, GREEN);
                    }
                } else if let Some((pi, t)) = self.pipe_at(at) {
                    let s = view.to_screen(v2f(self.mech.plumbing.pipes[pi].point(t.clamp(0.08, 0.92))));
                    draw_circle(s.x, s.y, 4.0 * view.s, Color::new(0.5, 1.0, 0.6, 0.8));
                    draw_circle_lines(s.x, s.y, 9.0 * view.s, 2.0, GREEN);
                }
            }
            Tool::Pipe => {
                let to = match self.pipe_from {
                    Some(from) => {
                        let to = self.pipe_end(from, at);
                        let (a, b) = (view.to_screen(from), view.to_screen(to));
                        let len = (to - from).length();
                        draw_line(a.x, a.y, b.x, b.y, PIPE_R * 2.0 * view.s, Color::new(0.55, 0.6, 0.75, 0.6));
                        ui.text(&format!("{len:.0} / {PIPE_MAX:.0}"), b.x + 10.0, b.y - 8.0, 15, if len >= PIPE_MIN { WHITE } else { RED });
                        to
                    }
                    None => self.pipe_snap(at),
                };
                let s = view.to_screen(to);
                let clipped = (to - at).length() > 0.5;
                draw_circle(s.x, s.y, 3.4 * view.s, if clipped { GREEN } else { WHITE });
                draw_circle_lines(s.x, s.y, 6.0 * view.s, 2.0, if clipped { GREEN } else { ACCENT });
            }
            Tool::Valve => {
                if let Some(i) = self.valve_at(at) {
                    if let Some(p) = self.valve_world(&self.mech.plumbing.valves[i]) {
                        let s = view.to_screen(p);
                        draw_circle_lines(s.x, s.y, 9.0 * view.s, 2.0, GREEN);
                    }
                } else if let Some((pi, t)) = self.pipe_at(at) {
                    let s = view.to_screen(v2f(self.mech.plumbing.pipes[pi].point(t.clamp(0.04, 0.96))));
                    draw_circle(s.x, s.y, 4.0 * view.s, Color::new(0.5, 1.0, 0.6, 0.8));
                    draw_circle_lines(s.x, s.y, 8.0 * view.s, 2.0, GREEN);
                }
            }
            Tool::Axle => {
                if let Some(i) = self.pump_at(at) {
                    if let Some(p) = self.pump_world(&self.mech.plumbing.pumps[i]) {
                        let s = view.to_screen(p);
                        draw_circle_lines(s.x, s.y, 11.0 * view.s, 2.0, GREEN);
                    }
                }
                if let Some(i) = self.valve_at(at) {
                    if let Some(p) = self.valve_world(&self.mech.plumbing.valves[i]) {
                        let s = view.to_screen(p);
                        draw_circle_lines(s.x, s.y, 9.0 * view.s, 2.0, GREEN);
                    }
                }
            }
            _ => {}
        }
    }

    /// Colour of liquid under a given head: the pressure there is the head minus the height.
    fn head_colour(&self, head: f32, p: &crate::pipes::Pipe, end: u8) -> Color {
        let q = if end == 0 { p.a } else { p.b };
        let gauge = head - (H as f32 - q.1);
        // A head of 60 cells of water is one atmosphere.
        pressure_colour(1.0 + gauge / 60.0)
    }
}

fn fluid_colour(m: Mat) -> Color {
    mat_color(m)
}

fn lerp_col(a: Color, b: Color, t: f32) -> Color {
    Color::new(a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t, 1.0)
}

/// Pressure (absolute, in atmospheres) as a colour, the same scale as the pressure view.
pub fn pressure_colour(atm: f32) -> Color {
    let (r, g, b) = world::pressure_rgb(atm);
    Color::from_rgba(r, g, b, 255)
}
