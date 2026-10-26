//! The simulation view: the world as the simulation holds it, layer by layer.
//!
//! A floating panel picks what is shown, in the terms of the code that computes it: a background
//! (the matter, the classes of the liquids, one of the pressure fields on its own grid, or what
//! the last step changed) and layers over it (the sleeping zones, the velocity of each kind of
//! matter in its own model, the drops and clods in flight, the rows the card works on). It also
//! reads out the cell under the pointer, and runs the world step by step: before each step the
//! state of every cell is kept, and the step's diff is what changed since.


use macroquad::prelude::*;
use rayon::prelude::*;

use crate::render::View;
use crate::science::info;
use crate::ui::{draw_icon, rrect, Icon, Ui, ACCENT, DIM, GREEN, TEXT};
use crate::world::inspect::{Field, Motion, Source, FULL};
use crate::world::liquid_class::{AIR, FULLISH, SOLID, WET};
use crate::world::sleep::{CH, CS, CW};
use crate::world::{self, Kind, Mat, World, H, W};

/// What a cell's step changed, for the diff.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiffOf {
    Matter,
    Fill,
    Temp,
    Pressure,
    Velocity,
}

impl DiffOf {
    pub const ALL: [DiffOf; 5] = [DiffOf::Matter, DiffOf::Fill, DiffOf::Temp, DiffOf::Pressure, DiffOf::Velocity];

    fn label(self) -> &'static str {
        match self {
            DiffOf::Matter => "matière",
            DiffOf::Fill => "liquide",
            DiffOf::Temp => "chaleur",
            DiffOf::Pressure => "pression",
            DiffOf::Velocity => "vitesse",
        }
    }

    /// The unit of the change, and how little counts as one.
    fn unit(self) -> (&'static str, f32) {
        match self {
            DiffOf::Matter => ("", 0.5),
            DiffOf::Fill => ("plein", 1e-4),
            DiffOf::Temp => ("°C", 0.01),
            DiffOf::Pressure => ("Pa", 0.5),
            DiffOf::Velocity => ("m/s", 0.01),
        }
    }
}

/// The picture under the layers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Back {
    /// The matter, dimmed.
    Matter,
    /// What each cell is to the liquids (`liquid.rs`).
    Classes,
    /// The pressure that pushes on walls: in the liquids and in the closed pockets of gas, Pa
    /// over the open air.
    Pressure,
    /// The air's pressure, from its flow (`flow.rs`), Pa over the ambient one.
    Air,
    /// What the last step changed.
    Diff(DiffOf),
}

/// A particle the game moves besides the world's cells (a piece of a wrecked car, a spark of a
/// firework): where it is (cells) and its velocity (m/s, down positive).
pub struct Particle {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    /// Its place in `PARTICLES`.
    pub kind: usize,
}

/// The kinds of `Particle`.
pub const PARTICLES: [(&str, Color); 2] = [("débris", Color::new(0.95, 0.75, 0.6, 0.9)), ("feux d'artifice", Color::new(1.0, 0.6, 0.9, 0.9))];

/// The kinds of matter, as the velocities are grouped.
const KINDS: [(&str, Color); 5] = [
    ("liquides", Color::new(0.75, 0.9, 1.0, 1.0)),
    ("grains", Color::new(1.0, 0.85, 0.45, 1.0)),
    ("gaz", Color::new(0.75, 1.0, 0.8, 1.0)),
    ("air", Color::new(0.75, 0.75, 0.8, 1.0)),
    ("solides", Color::new(1.0, 0.55, 0.3, 1.0)),
];

/// The group of `KINDS` a matter's velocity goes in.
fn group(m: Mat) -> usize {
    match m.kind() {
        Kind::Liquid => 0,
        Kind::Powder => 1,
        Kind::Gas | Kind::Flame => 2,
        Kind::Empty => 3,
        Kind::Static => 4,
    }
}

/// What the world holds in every cell, at one step: what the diff compares.
pub struct Snapshot {
    pub step: u32,
    mat: Vec<Mat>,
    /// Units of liquid (0 in a cell that is not liquid).
    fill: Vec<u32>,
    temp: Vec<f32>,
    /// The pressure of the liquid, Pa over the ambient one (NaN: none).
    pa: Vec<f32>,
    /// The velocity, m/s (NaN: none).
    vel: Vec<(f32, f32)>,
}

impl Snapshot {
    pub fn take(w: &World) -> Snapshot {
        let solids = w.solid_motions();
        let at = |i: usize| (i % W, i / W);
        let pa = |i: usize| {
            let (x, y) = at(i);
            match (w.field_value(Field::LiquidPa, x, y), w.field_value(Field::LiquidAtm, x, y)) {
                (Some(p), _) => p,
                (None, Some(atm)) => (atm - 1.0) * 101_325.0,
                _ => f32::NAN,
            }
        };
        Snapshot {
            step: w.steps(),
            mat: (0..W * H).map(|i| w.cells[i].m).collect(),
            fill: (0..W * H).map(|i| w.cell_fill(i % W, i / W).unwrap_or(0)).collect(),
            temp: w.temp.clone(),
            pa: (0..W * H).into_par_iter().map(pa).collect(),
            vel: (0..W * H)
                .into_par_iter()
                .map(|i| {
                    let (x, y) = at(i);
                    // The air everywhere is the flow's: only matter's velocity is compared.
                    match w.cells[i].m.kind() {
                        Kind::Empty => (f32::NAN, f32::NAN),
                        _ => w.cell_motion(x, y, &solids).map_or((0.0, 0.0), |m| (m.vx, m.vy)),
                    }
                })
                .collect(),
        }
    }

    /// The change of cell `i` from `self` to `after` (signed; for the matter, 1 if it changed).
    fn delta(&self, after: &Snapshot, of: DiffOf, i: usize) -> f32 {
        match of {
            DiffOf::Matter => (self.mat[i] != after.mat[i]) as u8 as f32,
            DiffOf::Fill => (after.fill[i] as f32 - self.fill[i] as f32) / FULL as f32,
            DiffOf::Temp => after.temp[i] - self.temp[i],
            DiffOf::Pressure => {
                // A cell that comes to hold a pressure, or no longer does, counts from 0.
                let z = |v: f32| if v.is_nan() { 0.0 } else { v };
                z(after.pa[i]) - z(self.pa[i])
            }
            DiffOf::Velocity => {
                let ((ax, ay), (bx, by)) = (self.vel[i], after.vel[i]);
                if ax.is_nan() || bx.is_nan() {
                    0.0
                } else {
                    ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt()
                }
            }
        }
    }
}

/// How much a step changed, for each `DiffOf`: the cells that changed and the largest change.
pub struct DiffStats {
    pub from: u32,
    pub to: u32,
    pub changed: [(usize, f32); 5],
}

pub struct SimView {
    pub back: Back,
    /// Which layers are drawn over it.
    pub zones: bool,
    pub card: bool,
    /// The liquids the solver of `liquid.rs` moves, and the gas it takes for a wall.
    pub solver: bool,
    /// That layer's picture, its revision, and how many cells it marks (full, film, drops left
    /// to the cells, gas taken for a wall).
    solver_image: Option<Image>,
    solver_texture: Option<Texture2D>,
    solver_key: Option<u64>,
    solver_counts: [usize; 4],
    pub flying: bool,
    /// The particles of the game in flight last drawn, by kind (`PARTICLES`).
    particles: [usize; 2],
    pub velocity: [bool; 5],
    /// Where the panel is (None: its place by default), and the drag that moves it.
    pub pos: Option<Vec2>,
    drag: Option<Vec2>,
    /// The panel on the screen, last drawn.
    pub rect: Rect,
    /// The world before and after the last steps taken one by one.
    before: Option<Snapshot>,
    after: Option<Snapshot>,
    pub stats: Option<DiffStats>,
    /// The cell under the pointer.
    hover: Option<(usize, usize)>,
    /// The background picture, and what it shows (revision, background, diff).
    image: Option<Image>,
    texture: Option<Texture2D>,
    key: Option<(u64, Back, u32)>,
    /// The values of the background, and the largest of them (its colours' scale).
    values: Vec<f32>,
    scale: f32,
    /// The largest of them.
    top: f32,
    /// Bumped at each new diff.
    serial: u32,
}

impl SimView {
    pub fn new() -> SimView {
        SimView {
            back: Back::Matter,
            zones: true,
            card: true,
            solver: true,
            solver_image: None,
            solver_texture: None,
            solver_key: None,
            solver_counts: [0; 4],
            flying: true,
            particles: [0; 2],
            velocity: [true, true, false, false, true],
            pos: None,
            drag: None,
            rect: Rect::default(),
            before: None,
            after: None,
            stats: None,
            hover: None,
            image: None,
            texture: None,
            key: None,
            values: vec![],
            scale: 1.0,
            top: 0.0,
            serial: 0,
        }
    }

    /// Before the world takes steps one by one: what it holds now.
    pub fn before_steps(&mut self, w: &World) {
        self.before = Some(Snapshot::take(w));
    }

    /// After them: the diff with what it held before.
    pub fn after_steps(&mut self, w: &World) {
        let Some(before) = &self.before else { return };
        let after = Snapshot::take(w);
        let mut changed = [(0usize, 0f32); 5];
        for (k, of) in DiffOf::ALL.iter().enumerate() {
            let least = of.unit().1;
            let (n, top) = (0..W * H)
                .into_par_iter()
                .map(|i| before.delta(&after, *of, i))
                .filter(|d| d.abs() >= least)
                .fold(|| (0usize, 0f32), |(n, t), d| (n + 1, t.max(d.abs())))
                .reduce(|| (0, 0.0), |a, b| (a.0 + b.0, a.1.max(b.1)));
            changed[k] = (n, top);
        }
        self.stats = Some(DiffStats { from: before.step, to: after.step, changed });
        self.after = Some(after);
        self.serial += 1;
        if !matches!(self.back, Back::Diff(_)) {
            self.back = Back::Diff(DiffOf::Matter);
        }
    }

    // ----- the picture ------------------------------------------------------

    /// The background, drawn where the world is.
    pub fn draw_back(&mut self, w: &World, view: View) {
        let key = (w.revision, self.back, self.serial);
        if self.key != Some(key) || self.texture.is_none() {
            self.rebuild(w);
            self.key = Some(key);
        }
        if let Some(t) = &self.texture {
            draw_texture_ex(t, view.ox, view.oy, WHITE, DrawTextureParams { dest_size: Some(vec2(W as f32 * view.s, H as f32 * view.s)), ..Default::default() });
        }
    }

    /// The solver's liquids over the background: full cells in blue, films in light blue, drops
    /// it leaves to the cells in yellow, and in orange a gas it takes for a wall beside its
    /// liquid (a cloud or steam in the open: the liquid rests on it as on the ground). Fainter
    /// where the calculation sleeps.
    fn draw_solver(&mut self, w: &World, view: View) {
        if self.solver_key != Some(w.revision) || self.solver_texture.is_none() {
            let image = self.solver_image.get_or_insert_with(|| Image::gen_image_color(W as u16, H as u16, BLANK));
            let px = image.get_image_data_mut();
            let class = |x: usize, y: usize| w.liquid_class(x, y).unwrap_or(SOLID);
            let liquid = |x: usize, y: usize| w.cell_mat(x, y).kind() == Kind::Liquid;
            let counts = px
                .par_chunks_mut(W)
                .enumerate()
                .map(|(y, row)| {
                    let mut n = [0usize; 4];
                    for (x, out) in row.iter_mut().enumerate() {
                        let c = class(x, y);
                        let m = w.cell_mat(x, y);
                        let a = |a: u8| if c & 4 != 0 { a } else { a / 2 };
                        *out = match (m.kind(), c & 3) {
                            (Kind::Liquid, FULLISH) => {
                                n[0] += 1;
                                [60, 140, 255, a(150)]
                            }
                            (Kind::Liquid, WET) => {
                                n[1] += 1;
                                [150, 215, 255, a(140)]
                            }
                            (Kind::Liquid, AIR) => {
                                n[2] += 1;
                                [255, 220, 60, 200]
                            }
                            (Kind::Gas, SOLID) if [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)].iter().any(|&(nx, ny)| nx < W && ny < H && liquid(nx, ny)) => {
                                n[3] += 1;
                                [255, 110, 0, 230]
                            }
                            _ => [0, 0, 0, 0],
                        };
                    }
                    n
                })
                .reduce(|| [0; 4], |a, b| [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]);
            self.solver_counts = counts;
            match &self.solver_texture {
                Some(t) => t.update(image),
                None => {
                    let t = Texture2D::from_image(image);
                    t.set_filter(FilterMode::Nearest);
                    self.solver_texture = Some(t);
                }
            }
            self.solver_key = Some(w.revision);
        }
        if let Some(t) = &self.solver_texture {
            draw_texture_ex(t, view.ox, view.oy, WHITE, DrawTextureParams { dest_size: Some(vec2(W as f32 * view.s, H as f32 * view.s)), ..Default::default() });
        }
    }

    fn rebuild(&mut self, w: &World) {
        let back = self.back;
        // The values first, for the scale of the colours.
        self.values.resize(W * H, f32::NAN);
        let (before, after) = (self.before.as_ref(), self.after.as_ref());
        self.values.par_iter_mut().enumerate().for_each(|(i, v)| {
            let (x, y) = (i % W, i / W);
            *v = match back {
                Back::Pressure => pressure_pa(w, x, y),
                Back::Air => w.field_value(Field::Air, x, y).map_or(f32::NAN, |p| p * ATM),
                Back::Diff(of) => match (before, after) {
                    (Some(b), Some(a)) => b.delta(a, of, i),
                    _ => f32::NAN,
                },
                _ => f32::NAN,
            };
        });
        // The scale is what 99 cells in 100 stay under, so that a few wild values do not flatten
        // the picture; those beyond ten times it show apart.
        let mut mags: Vec<f32> = self.values.par_iter().filter(|v| !v.is_nan() && **v != 0.0).map(|v| v.abs()).collect();
        self.top = mags.iter().copied().fold(0.0, f32::max);
        self.scale = if mags.is_empty() {
            1.0
        } else {
            let k = (mags.len() * 99 / 100).min(mags.len() - 1);
            *mags.select_nth_unstable_by(k, f32::total_cmp).1
        };
        let image = self.image.get_or_insert_with(|| Image::gen_image_color(W as u16, H as u16, BLACK));
        let (values, scale, top) = (&self.values, self.scale, self.top);
        let after = self.after.as_ref();
        image.get_image_data_mut().par_iter_mut().enumerate().for_each(|(i, px)| {
            let (x, y) = (i % W, i / W);
            let dim = |k: u32| {
                let c = w.color(x, y);
                let a = c[3] as u32;
                let f = |c: u8| (16 + c as u32 * a * k / (255 * 8)) as u8;
                [f(c[0]), f(c[1]), f(c[2]), 255]
            };
            *px = match back {
                Back::Matter => dim(5),
                Back::Classes => class_color(w, x, y, dim(3)),
                Back::Pressure | Back::Air | Back::Diff(DiffOf::Fill | DiffOf::Temp | DiffOf::Pressure) => {
                    let v = values[i];
                    if v.is_nan() || (matches!(back, Back::Diff(_)) && v == 0.0) {
                        dim(2)
                    } else if matches!(back, Back::Pressure | Back::Air) {
                        diverging(log_scale(v, top))
                    } else if v.abs() > 10.0 * scale {
                        WILD
                    } else {
                        diverging(v / scale)
                    }
                }
                Back::Diff(DiffOf::Velocity) => {
                    let v = values[i];
                    if v.is_nan() || v < DiffOf::Velocity.unit().1 {
                        dim(2)
                    } else {
                        let t = (v / scale).sqrt();
                        [(60.0 + 195.0 * t) as u8, (60.0 + 160.0 * t) as u8, (90.0 - 40.0 * t) as u8, 255]
                    }
                }
                Back::Diff(DiffOf::Matter) => {
                    if values[i] > 0.5 {
                        // What came: its own colour, bright; what was left empty, dark red.
                        match after.map(|a| a.mat[i]) {
                            Some(Mat::Empty) | None => [150, 30, 40, 255],
                            Some(_) => {
                                let c = w.color(x, y);
                                [c[0].max(90), c[1].max(90), c[2].max(90), 255]
                            }
                        }
                    } else {
                        dim(2)
                    }
                }
            };
        });
        match &self.texture {
            Some(t) => t.update(image),
            None => {
                let t = Texture2D::from_image(image);
                t.set_filter(FilterMode::Nearest);
                self.texture = Some(t);
            }
        }
    }

    /// The layers over the background, and the cell under the pointer.
    pub fn draw_layers(&mut self, w: &World, view: View, ui: &Ui, particles: &[Particle]) {
        let area = view.visible();
        let (a, b) = (view.to_world(vec2(area.x, area.y)), view.to_world(vec2(area.x + area.w, area.y + area.h)));
        let vis = Rect::new(a.x, a.y, b.x - a.x, b.y - a.y);
        let p = view.to_world(ui.mouse);
        self.hover = (!ui.over_ui() && area.contains(ui.mouse) && p.x >= 0.0 && p.y >= 0.0 && (p.x as usize) < W && (p.y as usize) < H).then(|| (p.x as usize, p.y as usize));
        if self.solver && w.liquid_physics {
            self.draw_solver(w, view);
        }
        if self.zones {
            for zy in 0..CH {
                for zx in 0..CW {
                    let r = Rect::new((zx * CS) as f32, (zy * CS) as f32, CS as f32, CS as f32);
                    if !vis.overlaps(&r) {
                        continue;
                    }
                    let a = view.to_screen(vec2(r.x, r.y));
                    let runs = w.zones.runs(zy * CW + zx);
                    let col = if runs { Color::new(0.3, 1.0, 0.4, 0.55) } else { Color::new(0.6, 0.6, 0.6, 0.18) };
                    draw_rectangle_lines(a.x, a.y, r.w * view.s, r.h * view.s, if runs { 1.5 } else { 1.0 }, col);
                }
            }
        }
        if self.card {
            if let Some((y0, y1)) = w.liquid_card_rows() {
                let col = Color::new(1.0, 0.6, 0.2, 0.8);
                for y in [y0 as f32, y1 as f32 + 1.0] {
                    let (a, b) = (view.to_screen(vec2(0.0, y)), view.to_screen(vec2(W as f32, y)));
                    draw_line(a.x, a.y, b.x, b.y, 1.5, col);
                }
                let a = view.to_screen(vec2(vis.x.max(0.0) + 2.0, y0 as f32));
                ui.text(&format!("carte : lignes {y0} à {y1}"), a.x.max(area.x + 4.0), a.y - 4.0, 12, col);
            }
        }
        self.draw_velocities(w, view, vis);
        if self.flying {
            let col = Color::new(1.0, 0.9, 0.3, 0.9);
            for d in w.liquid_drops() {
                let (vx, vy) = World::drop_velocity(d);
                let a = view.to_screen(vec2(d.x, d.y));
                arrow(a, vec2(vx, vy) * view.s, col);
                draw_circle(a.x, a.y, (2.0 * view.s.max(0.6)).min(5.0), col);
            }
            let col = Color::new(1.0, 0.6, 0.35, 0.9);
            for (x, y, vx, vy) in w.ejecta_motion() {
                let a = view.to_screen(vec2(x, y));
                arrow(a, vec2(vx, vy) * view.s, col);
                draw_rectangle(a.x - 2.0, a.y - 2.0, 4.0, 4.0, col);
            }
            for p in particles {
                let col = PARTICLES[p.kind].1;
                let a = view.to_screen(vec2(p.x, p.y));
                arrow(a, vec2(p.vx, p.vy) * view.s, col);
                draw_circle_lines(a.x, a.y, 2.5, 1.2, col);
            }
        }
        self.particles = particles.iter().fold([0; 2], |mut n, p| {
            n[p.kind] += 1;
            n
        });
        if let Some((x, y)) = self.hover {
            let a = view.to_screen(vec2(x as f32, y as f32));
            draw_rectangle_lines(a.x - 1.0, a.y - 1.0, view.s + 2.0, view.s + 2.0, 1.5, WHITE);
        }
    }

    /// One arrow per cell close up, sparser from afar; the air's on the flow's own cells.
    fn draw_velocities(&self, w: &World, view: View, vis: Rect) {
        if !self.velocity.iter().any(|&v| v) {
            return;
        }
        let solids = w.solid_motions();
        let x_range = |step: usize| ((vis.x.max(0.0) as usize / step * step)..((vis.x + vis.w).max(0.0) as usize + 1).min(W)).step_by(step);
        let y_range = |step: usize| ((vis.y.max(0.0) as usize / step * step)..((vis.y + vis.h).max(0.0) as usize + 1).min(H)).step_by(step);
        // One arrow a block of `step` × `step` cells: its fastest cell, so that a few grains
        // in flight are not missed between the samples.
        let step = (6.0 / view.s).ceil().max(1.0) as usize;
        for by in y_range(step) {
            for bx in x_range(step) {
                let mut best: Option<(f32, usize, usize, Motion, usize)> = None;
                for y in by..(by + step).min(H) {
                    for x in bx..(bx + step).min(W) {
                        let m = w.cell_mat(x, y);
                        let g = group(m);
                        if g == 3 || !self.velocity[g] || (g == 4 && solids.is_empty()) {
                            continue;
                        }
                        if let Some(mo) = w.cell_motion(x, y, &solids) {
                            let speed = mo.vx * mo.vx + mo.vy * mo.vy;
                            if best.as_ref().map_or(true, |b| speed > b.0) {
                                best = Some((speed, x, y, mo, g));
                            }
                        }
                    }
                }
                if let Some((_, x, y, mo, g)) = best {
                    draw_motion(view, x as f32 + 0.5, y as f32 + 0.5, mo, KINDS[g].1);
                }
            }
        }
        if self.velocity[3] {
            // The air: at the middle of each flow cell (every other one from afar).
            let c = world::flow::CELL;
            let every = if c as f32 * view.s < 10.0 { 2 } else { 1 };
            for y in y_range(c * every) {
                for x in x_range(c * every) {
                    let (cx, cy) = (x + c / 2, (y + c / 2).min(H - 1));
                    if cx >= W || w.cell_mat(cx, cy).kind() != Kind::Empty {
                        continue;
                    }
                    if let Some(m) = w.cell_motion(cx, cy, &solids) {
                        draw_motion(view, cx as f32, cy as f32, m, Color { a: 0.6, ..KINDS[3].1 });
                    }
                }
            }
        }
    }

    // ----- the panel --------------------------------------------------------

    /// The floating panel. Returns what it asks of the game: steps to take, or a pause.
    pub fn panel(&mut self, ui: &mut Ui, w: &World, running: bool, sw: f32, sh: f32) -> Ask {
        const PW: f32 = 300.0;
        let mut ask = Ask::None;
        let lines = self.readout(w);
        // As tall as what it held last frame (its content changes with the choices).
        let h = self.rect.h.max(200.0);
        let pos = self.pos.unwrap_or(vec2(sw - 22.0 - PW - 10.0, 52.0));
        let pos = vec2(pos.x.clamp(0.0, (sw - PW).max(0.0)), pos.y.clamp(0.0, (sh - 30.0).max(0.0)));
        let r = Rect::new(pos.x, pos.y, PW, h);
        self.rect = r;
        ui.layer("simulation");
        ui.claim("simulation", r);
        rrect(r, 12.0, Color::new(0.06, 0.07, 0.13, 0.93));
        // The title bar moves the panel.
        let bar = Rect::new(r.x, r.y, r.w, 28.0);
        if ui.pressed && bar.contains(ui.mouse) {
            self.drag = Some(ui.mouse - pos);
        }
        if let Some(off) = self.drag {
            if is_mouse_button_down(MouseButton::Left) {
                self.pos = Some(ui.mouse - off);
            } else {
                self.drag = None;
            }
        }
        draw_icon(Icon::Science, vec2(r.x + 16.0, r.y + 15.0), 14.0, GREEN);
        ui.text("Simulation", r.x + 30.0, r.y + 20.0, 15, TEXT);
        ui.text(&format!("pas {}", w.steps()), r.x + r.w - 12.0 - ui.text_w(&format!("pas {}", w.steps()), 12), r.y + 19.0, 12, DIM);
        let (x, cw) = (r.x + 10.0, r.w - 20.0);
        let mut y = r.y + 32.0;

        // Step by step.
        let bw = (cw - 12.0) / 3.0;
        if ui.button(Rect::new(x, y, bw, 26.0), if running { "Pause" } else { "Lancer" }, if running { Icon::Pause } else { Icon::Play }, running, true, ACCENT) {
            ask = Ask::Run(!running);
        }
        if ui.button(Rect::new(x + bw + 6.0, y, bw, 26.0), "1 pas", Icon::Plus, false, true, ACCENT) {
            ask = Ask::Steps(1);
        }
        if ui.button(Rect::new(x + 2.0 * (bw + 6.0), y, bw, 26.0), "10 pas", Icon::Plus, false, true, ACCENT) {
            ask = Ask::Steps(10);
        }
        y += 32.0;
        let note = match &self.stats {
            Some(s) => format!("Les « Changements » comparent le pas {} au pas {}. Touche point : un pas.", s.from, s.to),
            None => "« 1 pas » avance le monde d'un pas puis le met en pause ; le fond « Changements » montre alors ce qu'il a modifié. Touche point : un pas.".to_string(),
        };
        y += ui.wrapped(&note, x, y + 10.0, cw, 11, DIM) + 4.0;

        // The background.
        ui.text("Le fond montre", x, y + 12.0, 13, ACCENT);
        y += 20.0;
        let diff = match self.back {
            Back::Diff(of) => of,
            _ => DiffOf::Matter,
        };
        let mut choices = vec![
            ("La matière", Back::Matter),
            ("La pression (liquides, poches de gaz)", Back::Pressure),
            ("La pression de l'air", Back::Air),
            ("Les changements du dernier pas", Back::Diff(diff)),
        ];
        // Only the liquids moving by their physics have classes.
        if w.liquid_physics {
            choices.insert(1, ("Le calcul des liquides (plein, film, goutte)", Back::Classes));
        } else if self.back == Back::Classes {
            self.back = Back::Matter;
        }
        for (label, b) in choices {
            let on = self.back == b || matches!((self.back, b), (Back::Diff(_), Back::Diff(_)));
            if ui.checkbox(Rect::new(x, y, cw, 20.0), label, on, true) {
                self.back = b;
            }
            y += 22.0;
        }
        if let Back::Diff(_) = self.back {
            let tw = cw / 5.0;
            for (k, of) in DiffOf::ALL.iter().enumerate() {
                let on = self.back == Back::Diff(*of);
                let b = Rect::new(x + k as f32 * tw, y, tw - 3.0, 22.0);
                ui.force_size = Some(11);
                if ui.button(b, of.label(), Icon::None, on, true, GREEN) {
                    self.back = Back::Diff(*of);
                }
                ui.force_size = None;
            }
            y += 26.0;
        }
        self.legend(ui, x, y, cw);
        y += 58.0;

        // The layers.
        ui.text("Flèches de vitesse (1 m/s = 1 case)", x, y + 12.0, 13, ACCENT);
        y += 18.0;
        let kw = cw / 5.0;
        for (k, (label, col)) in KINDS.iter().enumerate() {
            let b = Rect::new(x + k as f32 * kw, y, kw - 3.0, 22.0);
            ui.force_size = Some(11);
            if ui.button(b, label, Icon::None, self.velocity[k], true, *col) {
                self.velocity[k] = !self.velocity[k];
            }
            ui.force_size = None;
        }
        y += 28.0;
        let flying = w.liquid_drops().len() + w.ejecta_motion().count() + self.particles[0] + self.particles[1];
        let card = w.liquid_card_rows().is_some();
        if w.liquid_physics {
            let [full, film, drops, walls] = self.solver_counts;
            if ui.checkbox(Rect::new(x, y, cw, 20.0), &format!("Liquides du solveur ({})", full + film), self.solver, true) {
                self.solver = !self.solver;
            }
            y += 22.0;
            if self.solver {
                let keys = [((60, 140, 255), format!("plein {}", count(full))), ((150, 215, 255), format!("film {}", count(film))), ((255, 220, 60), format!("goutte {}", count(drops)))];
                for (k, (c, label)) in keys.iter().enumerate() {
                    let bx = x + 8.0 + k as f32 * (cw - 8.0) / 3.0;
                    draw_rectangle(bx, y + 2.0, 9.0, 9.0, Color::from_rgba(c.0, c.1, c.2, 255));
                    ui.text(label, bx + 13.0, y + 11.0, 11, DIM);
                }
                draw_rectangle(x + 8.0, y + 17.0, 9.0, 9.0, Color::from_rgba(255, 110, 0, 255));
                ui.text(&format!("gaz pris pour une paroi : {} cases", count(walls)), x + 21.0, y + 26.0, 11, if walls > 0 { TEXT } else { DIM });
                y += 32.0;
            }
        }
        let mut layers = vec![
            (format!("Zones de calcul actives ({})", w.zones.awake_count()), &mut self.zones),
            (format!("Particules en vol ({flying})"), &mut self.flying),
        ];
        if card {
            layers.push(("Lignes que la carte graphique calcule".to_string(), &mut self.card));
        }
        for (label, on) in layers {
            if ui.checkbox(Rect::new(x, y, cw, 20.0), &label, *on, true) {
                *on = !*on;
            }
            y += 22.0;
        }
        y += 4.0;

        // The cell under the pointer.
        ui.text("La case sous la souris", x, y + 12.0, 13, ACCENT);
        y += 14.0;
        for (k, (l, col)) in lines.iter().enumerate() {
            ui.text(l, x, y + 12.0 + k as f32 * 15.0, 12, *col);
        }
        self.rect.h = y + 14.0 + lines.len() as f32 * 15.0 + 10.0 - r.y;
        ask
    }

    fn legend(&self, ui: &Ui, x: f32, y: f32, cw: f32) {
        let ramp = |ui: &Ui, lo: String, hi: String, f: &dyn Fn(f32) -> [u8; 4]| {
            for k in 0..60 {
                let c = f(k as f32 / 59.0 * 2.0 - 1.0);
                draw_rectangle(x + k as f32 * cw / 60.0, y, cw / 60.0 + 0.5, 10.0, Color::from_rgba(c[0], c[1], c[2], 255));
            }
            ui.text(&lo, x, y + 24.0, 11, DIM);
            ui.text(&hi, x + cw - ui.text_w(&hi, 11), y + 24.0, 11, DIM);
        };
        match self.back {
            Back::Matter => {
                ui.wrapped("Le monde, atténué, pour lire les flèches et les zones par-dessus.", x, y + 10.0, cw, 11, DIM);
            }
            Back::Classes => {
                for (k, (c, label)) in [((40, 96, 230), "plein"), ((130, 200, 245), "film"), ((245, 220, 70), "goutte"), ((96, 84, 60), "paroi")].iter().enumerate() {
                    let bx = x + k as f32 * cw / 4.0;
                    draw_rectangle(bx, y, 10.0, 10.0, Color::from_rgba(c.0, c.1, c.2, 255));
                    ui.text(label, bx + 14.0, y + 10.0, 11, DIM);
                }
                ui.wrapped("Un liquide plein porte une pression, un film suit le courant, une goutte tombe seule. Plus sombre : le calcul dort là.", x, y + 24.0, cw, 10, DIM);
            }
            Back::Pressure | Back::Air => {
                if self.top == 0.0 {
                    let text = if self.back == Back::Pressure { "Rien n'est sous pression : ni liquide, ni poche de gaz fermée en vue." } else { "L'air est partout à la pression ambiante." };
                    ui.wrapped(text, x, y + 10.0, cw, 11, DIM);
                } else {
                    // A logarithmic scale: a pond (a few kPa) and a closed pocket (hundreds) both show.
                    let top = self.top;
                    ramp(ui, format!("{} : moins que l'air libre", pa(-top)), format!("plus : {}", pa(top)), &|t| diverging(t));
                    for p in [100.0, 1000.0, 10_000.0, 100_000.0] {
                        if p < top {
                            for sign in [-1.0, 1.0] {
                                let t = log_scale(sign * p, top);
                                let tx = x + (t + 1.0) / 2.0 * cw;
                                draw_line(tx, y, tx, y + 10.0, 1.0, Color::new(0.0, 0.0, 0.0, 0.6));
                            }
                            if p < 1000.0 {
                                continue;
                            }
                            let tx = x + (log_scale(p, top) + 1.0) / 2.0 * cw;
                            let label = pa(p).trim_start_matches('+').to_string();
                            ui.text(&label, tx - ui.text_w(&label, 9) / 2.0, y + 36.0, 9, DIM);
                        }
                    }
                }
            }
            Back::Diff(of) => {
                let (unit, _) = of.unit();
                let s = self.scale;
                match of {
                    DiffOf::Matter => {
                        draw_rectangle(x, y, 10.0, 10.0, Color::from_rgba(150, 30, 40, 255));
                        ui.text("vidée", x + 14.0, y + 10.0, 11, DIM);
                        draw_rectangle(x + 70.0, y, 10.0, 10.0, Color::from_rgba(220, 200, 150, 255));
                        ui.text("nouvelle matière (sa couleur)", x + 84.0, y + 10.0, 11, DIM);
                    }
                    DiffOf::Velocity => ramp(ui, "0".into(), format!("{s:.2} {unit}"), &|t| {
                        let t = ((t + 1.0) / 2.0).sqrt();
                        [(60.0 + 195.0 * t) as u8, (60.0 + 160.0 * t) as u8, (90.0 - 40.0 * t) as u8, 255]
                    }),
                    _ => ramp(ui, format!("{:+.3} {unit}", -s), format!("{:+.3} {unit}", s), &|t| diverging(t)),
                }
                if of != DiffOf::Matter && self.stats.is_some() {
                    self.wild(ui, x, y + 50.0, &format!("{:.3} {unit}", self.top));
                }
                if self.stats.is_none() {
                    ui.text("Pas encore de pas fait ici : bouton « 1 pas ».", x, y + 38.0, 10, DIM);
                } else if let Some(st) = &self.stats {
                    let k = DiffOf::ALL.iter().position(|o| *o == of).unwrap_or(0);
                    let (n, top) = st.changed[k];
                    let text = if of == DiffOf::Matter { format!("{} cases ont changé de matière", count(n)) } else { format!("{} cases, au plus {top:.3} {unit}", count(n)) };
                    ui.text(&text, x, y + 38.0, 10, DIM);
                }
            }
        }
    }

    /// Under a ramp: its scale holds 99 cells in 100, and the largest value, flagged when wild.
    fn wild(&self, ui: &Ui, x: f32, y: f32, top: &str) {
        let far = self.top > 10.0 * self.scale;
        if far {
            draw_rectangle(x, y - 8.0, 9.0, 9.0, Color::from_rgba(WILD[0], WILD[1], WILD[2], 255));
        }
        let text = if far { format!("plus de 10 fois l'échelle (99 % des cases) ; max {top}") } else { format!("échelle : 99 % des cases ; max {top}") };
        ui.text(&text, x + if far { 13.0 } else { 0.0 }, y, 10, if far { Color::from_rgba(255, 140, 255, 255) } else { DIM });
    }

    /// What the simulation holds in the cell under the pointer, a line each.
    fn readout(&self, w: &World) -> Vec<(String, Color)> {
        let Some((x, y)) = self.hover else {
            return vec![("(pointe une case du monde)".into(), DIM)];
        };
        let i = y * W + x;
        let m = w.cell_mat(x, y);
        let zone = (y / CS) * CW + x / CS;
        let mut out = vec![(format!("case ({x}, {y}) · zone {zone} {}", if w.zones.runs(zone) { "tourne" } else { "dort" }), TEXT)];
        let kind = match m.kind() {
            Kind::Empty => "vide",
            Kind::Static => "solide",
            Kind::Powder => "granulaire",
            Kind::Liquid => "liquide",
            Kind::Gas => "gaz",
            Kind::Flame => "flamme",
        };
        out.push((format!("{} ({kind}) · {:.1} °C", info(m).name, w.temp[i]), TEXT));
        if let Some(c) = w.liquid_class(x, y) {
            let name = match c & 3 {
                FULLISH => "plein",
                WET => "film",
                AIR => "air",
                _ => "paroi",
            };
            let fill = w.cell_fill(x, y).map_or(String::new(), |f| format!(" · rempli à {:.3}", f as f32 / FULL as f32));
            out.push((format!("classe liquides : {name}{fill}"), DIM));
        } else if let Some(f) = w.cell_fill(x, y) {
            out.push((format!("rempli à {:.3}", f as f32 / FULL as f32), DIM));
        }
        let fall = w.cell_fall(x, y);
        if fall != 0 {
            out.push((format!("vel = {fall} (huitièmes de case par pas)"), DIM));
        }
        let solids = w.solid_motions();
        match w.cell_motion(x, y, &solids) {
            Some(Motion { vx, vy, source }) => {
                out.push((format!("vitesse {:.2} m/s ({vx:+.2}, {vy:+.2})", (vx * vx + vy * vy).sqrt()), TEXT));
                out.push((format!("  de : {}", source.label()), DIM));
            }
            None if m.kind() == Kind::Static => out.push(("vitesse : aucune (solide immobile)".into(), DIM)),
            None => {}
        }
        let p = pressure_pa(w, x, y);
        if !p.is_nan() {
            let from = if w.field_value(Field::LiquidPa, x, y).is_some() { "liquid.rs" } else if m.kind() == Kind::Liquid { "pockets.rs" } else { "poche fermée" };
            out.push((format!("pression : {} sur l'air libre ({from})", pa(p)), TEXT));
        }
        if let Some(a) = w.field_value(Field::Air, x, y) {
            out.push((format!("pression de l'air : {} (flow.rs)", pa(a * ATM)), DIM));
        }
        if let Some((k, cells, p, open)) = w.pocket_info(x, y) {
            out.push((format!("poche de gaz {k} : {cells} cases, {p:.3} atm, {}", if open { "ouverte" } else { "fermée" }), DIM));
        }
        // The same cell at the step before.
        if let (Some(b), Some(a)) = (&self.before, &self.after) {
            let mut was = vec![];
            if b.mat[i] != a.mat[i] {
                was.push(info(b.mat[i]).name.to_string());
            }
            if b.fill[i] != a.fill[i] {
                was.push(format!("rempli à {:.3}", b.fill[i] as f32 / FULL as f32));
            }
            if (b.temp[i] - a.temp[i]).abs() >= 0.01 {
                was.push(format!("{:.2} °C", b.temp[i]));
            }
            if !b.pa[i].is_nan() && (b.pa[i] - a.pa[i]).abs() >= 0.5 {
                was.push(format!("{:+.1} Pa", b.pa[i]));
            }
            let (vx, vy) = b.vel[i];
            if !vx.is_nan() && b.delta(a, DiffOf::Velocity, i) >= 0.01 {
                was.push(format!("{:.2} m/s", (vx * vx + vy * vy).sqrt()));
            }
            let text = if was.is_empty() { format!("au pas {} : pareil", b.step) } else { format!("au pas {} : {}", b.step, was.join(", ")) };
            out.push((text, ACCENT));
        }
        out
    }
}

/// What the panel asks of the game.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ask {
    None,
    /// Start or pause the world.
    Run(bool),
    /// Take that many steps, then stay paused.
    Steps(usize),
}

/// One atmosphere, Pa.
const ATM: f32 = 101_325.0;

/// The pressure that pushes on walls at (x, y), Pa over the open air: in a liquid (by its physics,
/// or the cells' own count), or in a closed pocket of gas; NaN elsewhere.
fn pressure_pa(w: &World, x: usize, y: usize) -> f32 {
    if let Some(p) = w.field_value(Field::LiquidPa, x, y) {
        return p;
    }
    if let Some(atm) = w.field_value(Field::LiquidAtm, x, y) {
        return (atm - 1.0) * ATM;
    }
    match w.pocket_info(x, y) {
        Some((_, _, p, false)) => (p - 1.0) * ATM,
        _ => f32::NAN,
    }
}

/// Where `v` falls on a logarithmic scale up to `top` either side of 0, as -1..1 (100 Pa makes
/// the first step).
fn log_scale(v: f32, top: f32) -> f32 {
    let l = |v: f32| (1.0 + v.abs() / 100.0).ln();
    v.signum() * l(v) / l(top).max(1e-6)
}

/// A pressure, in Pa or kPa.
fn pa(v: f32) -> String {
    if v.abs() >= 10_000.0 {
        format!("{:+.0} kPa", v / 1000.0)
    } else if v.abs() >= 1000.0 {
        format!("{:+.1} kPa", v / 1000.0)
    } else {
        format!("{:+.0} Pa", v)
    }
}

/// A value more than ten times the scale of the colours.
const WILD: [u8; 4] = [255, 40, 255, 255];

/// A number of cells, short.
fn count(n: usize) -> String {
    if n >= 10_000 {
        format!("{}k", n / 1000)
    } else {
        n.to_string()
    }
}

/// Blue (below) through grey to red (above), for t in -1..1 (a square root, so that small
/// values show).
fn diverging(t: f32) -> [u8; 4] {
    let t = t.clamp(-1.0, 1.0);
    let s = t.abs().sqrt();
    let (to, mid) = if t >= 0.0 { ([230.0, 60.0, 45.0], [70.0, 70.0, 78.0]) } else { ([45.0, 110.0, 240.0], [70.0, 70.0, 78.0]) };
    let c = |k: usize| (mid[k] + (to[k] - mid[k]) * s) as u8;
    [c(0), c(1), c(2), 255]
}

/// What a cell is to the liquids: full in blue, a film or spray in light blue, a lone drop left
/// to the cells in yellow, a wall (a solid, or a gas in the open) in grey, the rest `dim`; a cell
/// whose zone sleeps is darker still.
fn class_color(w: &World, x: usize, y: usize, dim: [u8; 4]) -> [u8; 4] {
    let Some(class) = w.liquid_class(x, y) else {
        return dim;
    };
    let runs = class & 4 != 0;
    let m = w.cell_mat(x, y);
    let col: [u8; 4] = match class & 3 {
        FULLISH => [40, 96, 230, 255],
        WET => [130, 200, 245, 255],
        AIR if m.kind() == Kind::Liquid => [245, 220, 70, 255],
        AIR => [16, 18, 28, 255],
        _ if m == Mat::Empty || m.kind() == Kind::Gas => [96, 84, 60, 255],
        _ => dim,
    };
    if runs || class & 3 < WET {
        col
    } else {
        [col[0] / 2, col[1] / 2, col[2] / 2, 255]
    }
}

/// An arrow from `a`, `d` pixels long (at most 60), with a head.
fn arrow(a: Vec2, d: Vec2, col: Color) {
    let len = d.length();
    if len < 0.5 {
        return;
    }
    let d = d * (len.min(60.0) / len);
    let b = a + d;
    draw_line(a.x, a.y, b.x, b.y, 1.2, col);
    if len > 4.0 {
        let dir = d.normalize();
        let side = vec2(-dir.y, dir.x);
        draw_triangle(b, b - dir * 4.0 + side * 2.2, b - dir * 4.0 - side * 2.2, col);
    }
}

/// A cell's velocity as an arrow from its middle (1 m/s = 1 cell); the faster, the brighter. The
/// mark at its root says which model it comes from: a dot for the liquids' grid, a square for a
/// fall, a ring for the wind, a cross for a solid.
fn draw_motion(view: View, x: f32, y: f32, m: Motion, col: Color) {
    let speed = (m.vx * m.vx + m.vy * m.vy).sqrt();
    if speed < 0.02 {
        return;
    }
    let c = view.to_screen(vec2(x, y));
    let col = Color { a: col.a * (0.4 + speed * 0.15).min(1.0), ..col };
    let len = (speed * view.s).max(3.0);
    arrow(c, vec2(m.vx, m.vy) / speed * len, col);
    match m.source {
        Source::LiquidGrid => draw_circle(c.x, c.y, 1.2, col),
        Source::Fall => draw_rectangle(c.x - 1.2, c.y - 1.2, 2.4, 2.4, col),
        Source::Wind => draw_circle_lines(c.x, c.y, 2.0, 1.0, col),
        Source::Jet => draw_triangle(c + vec2(0.0, -2.2), c + vec2(-2.0, 1.6), c + vec2(2.0, 1.6), col),
        Source::Piece | Source::Slab => {
            draw_line(c.x - 2.0, c.y - 2.0, c.x + 2.0, c.y + 2.0, 1.0, col);
            draw_line(c.x - 2.0, c.y + 2.0, c.x + 2.0, c.y - 2.0, 1.0, col);
        }
    }
}
