mod blocks;
mod car;
mod render;
mod save;
mod ui;
mod world;

use blocks::{block_at, placement_ok, snap, stamp_blocks, Block, BlockKind, Physics};
use car::{Car, Fate, ModelKind};
use macroquad::prelude::*;
use render::{draw_block, draw_block_shape, draw_booms, draw_car, draw_flag, draw_sky, Boom, Renderer, View};
use ui::{draw_icon, rrect, Icon, Ui, ACCENT, BG, BLUE, DIM, GREEN, PANEL, RED, TEXT};
use world::{Cell, Mat, World, H, W};

const PANEL_W: f32 = 236.0;
const TOP_H: f32 = 68.0;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Matter,
    Blocks,
    Cars,
    Tools,
}

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Mat(Mat),
    Block(BlockKind),
    Eraser,
    Rotate,
    Entry,
    Exit,
}

const MATTER: [(Mat, &str); 10] = [
    (Mat::Sand, "Sable"),
    (Mat::Water, "Eau"),
    (Mat::Lava, "Lave"),
    (Mat::Fire, "Feu"),
    (Mat::Steam, "Vapeur"),
    (Mat::Ice, "Glace"),
    (Mat::Gunpowder, "Poudre"),
    (Mat::Iron, "Fer"),
    (Mat::Wood, "Bois"),
    (Mat::Stone, "Pierre"),
];

fn mat_rgb(m: Mat) -> Color {
    let (r, g, b) = match m {
        Mat::Sand => (226, 200, 124),
        Mat::Water => (44, 124, 224),
        Mat::Lava => (255, 90, 14),
        Mat::Fire => (255, 150, 30),
        Mat::Steam => (205, 214, 230),
        Mat::Ice => (176, 226, 248),
        Mat::Gunpowder => (66, 62, 74),
        Mat::Iron => (146, 156, 174),
        Mat::Wood => (150, 100, 54),
        Mat::Stone => (110, 110, 122),
        _ => (128, 128, 128),
    };
    Color::from_rgba(r, g, b, 255)
}

fn window_conf() -> Conf {
    Conf {
        window_title: "AWorld".to_owned(),
        window_width: (PANEL_W + W as f32 * 2.0) as i32,
        window_height: (TOP_H + H as f32 * 2.0) as i32,
        window_resizable: true,
        sample_count: 4,
        ..Default::default()
    }
}

struct Game {
    world: World,
    snap_cells: Vec<Cell>,
    snap_blocks: Vec<Block>,
    blocks: Vec<Block>,
    phys: Option<Physics>,
    cars: Vec<Car>,
    entry: Option<(i32, i32)>,
    exit: Option<(i32, i32)>,
    running: bool,
    tool: Tool,
    tab: Tab,
    brush: i32,
    rot90: u8,
    speed: usize,
    model: ModelKind,
    arrived: u32,
    lost: u32,
    seed: u32,
    slot: usize,
    last: Option<Vec2>,
    toast: Option<(String, f32)>,
    booms: Vec<Boom>,
    time: f32,
    fullscreen: bool,
}

impl Game {
    fn new() -> Self {
        let mut world = World::new();
        let (entry, exit) = world.generate(1);
        Game {
            snap_cells: world.snapshot(),
            snap_blocks: vec![],
            world,
            blocks: vec![],
            phys: None,
            cars: vec![],
            entry: Some(entry),
            exit: Some(exit),
            running: false,
            tool: Tool::Mat(Mat::Sand),
            tab: Tab::Matter,
            brush: 6,
            rot90: 0,
            speed: 2,
            model: ModelKind::Buggy,
            arrived: 0,
            lost: 0,
            seed: 1,
            slot: 0,
            last: None,
            toast: None,
            booms: vec![],
            time: 0.0,
            fullscreen: false,
        }
    }

    fn say(&mut self, s: impl Into<String>) {
        self.toast = Some((s.into(), 2.5));
    }

    fn refresh_mask(&mut self) {
        stamp_blocks(&mut self.world, &self.blocks);
    }

    fn set_running(&mut self, run: bool) {
        if run && !self.running {
            self.snap_cells = self.world.snapshot();
            self.snap_blocks = self.blocks.iter().map(|b| Block { body: None, ..b.clone() }).collect();
            if self.phys.is_none() {
                self.phys = Some(Physics::build(&mut self.blocks));
            }
        }
        self.running = run;
    }

    fn reset(&mut self) {
        self.world.restore(&self.snap_cells);
        self.blocks = self.snap_blocks.clone();
        self.phys = None;
        self.cars.clear();
        self.running = false;
        self.refresh_mask();
    }

    fn fresh(&mut self) {
        self.blocks.clear();
        self.phys = None;
        self.cars.clear();
        self.running = false;
        self.arrived = 0;
        self.lost = 0;
        self.refresh_mask();
        self.snap_cells = self.world.snapshot();
        self.snap_blocks.clear();
    }

    fn new_world(&mut self) {
        self.seed += 1;
        let (e, x) = self.world.generate(self.seed);
        self.entry = Some(e);
        self.exit = Some(x);
        self.fresh();
    }

    fn empty_world(&mut self) {
        self.world.clear();
        self.entry = None;
        self.exit = None;
        self.fresh();
    }

    fn launch_car(&mut self) {
        if let Some(e) = self.entry {
            if self.cars.len() < 8 {
                self.cars.push(Car::new(self.model, e));
            }
            self.set_running(true);
        } else {
            self.say("Pose d'abord une Entrée (onglet Outils)");
        }
    }

    fn save(&mut self) {
        match save::save_slot(self.slot, &self.world, self.entry, self.exit, &self.blocks) {
            Ok(()) => self.say(format!("Monde sauvegardé (emplacement {})", self.slot + 1)),
            Err(e) => self.say(format!("Échec de la sauvegarde : {e}")),
        }
    }

    fn load(&mut self) {
        match save::load_slot(self.slot) {
            Ok(l) => {
                self.world.clear();
                self.world.load_mats(&l.mats);
                self.entry = l.entry;
                self.exit = l.exit;
                self.fresh();
                self.blocks = l.blocks;
                self.snap_blocks = self.blocks.clone();
                self.refresh_mask();
                self.say(format!("Monde chargé (emplacement {})", self.slot + 1));
            }
            Err(e) => self.say(format!("Impossible de charger : {e}")),
        }
    }

    fn toggle_fullscreen(&mut self) {
        self.fullscreen = !self.fullscreen;
        set_fullscreen(self.fullscreen);
    }

    fn rotate_preview(&mut self, d: i32) {
        self.rot90 = ((self.rot90 as i32 + d).rem_euclid(4)) as u8;
    }

    // ----- block editing ----------------------------------------------------

    fn candidate(&self, kind: BlockKind, at: Vec2) -> Block {
        let pos = if is_key_down(KeyCode::LeftShift) { at } else { snap(kind, self.rot90, at) };
        Block::new(kind, pos, self.rot90 as f32 * std::f32::consts::FRAC_PI_2)
    }

    fn place_block(&mut self, kind: BlockKind, at: Vec2) {
        let mut b = self.candidate(kind, at);
        if !placement_ok(&self.world, &self.blocks, &b, None) {
            return;
        }
        if let Some(p) = &mut self.phys {
            p.add_block(&mut b);
        }
        self.blocks.push(b);
        let i = self.blocks.len() - 1;
        if let Some(p) = &mut self.phys {
            p.weld_neighbours(&self.blocks, i);
        }
        self.refresh_mask();
    }

    fn remove_block(&mut self, i: usize) {
        let mut b = self.blocks.remove(i);
        if let Some(p) = &mut self.phys {
            p.remove_block(&mut b);
        }
        self.refresh_mask();
    }

    fn rotate_block(&mut self, i: usize) {
        let mut b = self.blocks[i].clone();
        b.angle += std::f32::consts::FRAC_PI_2;
        if !placement_ok(&self.world, &self.blocks, &b, Some(i)) {
            self.say("Pas la place pour tourner ce bloc");
            return;
        }
        if let Some(p) = &mut self.phys {
            p.remove_block(&mut self.blocks[i]);
            b.body = None;
            p.add_block(&mut b);
        }
        self.blocks[i] = b;
        if let Some(p) = &mut self.phys {
            p.weld_neighbours(&self.blocks, i);
        }
        self.refresh_mask();
    }

    fn burn_block(&mut self, i: usize) {
        let b = self.blocks[i].clone();
        let (x0, y0, x1, y1) = b.bounds();
        self.remove_block(i);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if b.covers(vec2(x as f32 + 0.5, y as f32 + 0.5)) && !self.world.raw(x, y).is_hard() {
                    self.world.set(x, y, Mat::Ember);
                }
            }
        }
    }

    // ----- input ------------------------------------------------------------

    fn handle_keys(&mut self) {
        if is_key_pressed(KeyCode::Space) {
            let run = !self.running;
            self.set_running(run);
        }
        if is_key_pressed(KeyCode::R) {
            self.reset();
        }
        if is_key_pressed(KeyCode::G) {
            self.new_world();
        }
        if is_key_pressed(KeyCode::N) {
            self.empty_world();
        }
        if is_key_pressed(KeyCode::C) {
            self.launch_car();
        }
        if is_key_pressed(KeyCode::T) || is_key_pressed(KeyCode::E) {
            self.rotate_preview(1);
        }
        if is_key_pressed(KeyCode::Q) {
            self.rotate_preview(-1);
        }
        if is_key_pressed(KeyCode::F11) || is_key_pressed(KeyCode::F) {
            self.toggle_fullscreen();
        }
        if is_key_pressed(KeyCode::F5) {
            self.save();
        }
        if is_key_pressed(KeyCode::F9) {
            self.load();
        }
        if is_key_pressed(KeyCode::Up) {
            self.speed = (self.speed + 1).min(6);
        }
        if is_key_pressed(KeyCode::Down) {
            self.speed = self.speed.saturating_sub(1).max(1);
        }
        if is_key_pressed(KeyCode::RightBracket) {
            self.brush = (self.brush + 1).min(40);
        }
        if is_key_pressed(KeyCode::LeftBracket) {
            self.brush = (self.brush - 1).max(0);
        }
        if is_key_pressed(KeyCode::Tab) {
            self.tab = match self.tab {
                Tab::Matter => Tab::Blocks,
                Tab::Blocks => Tab::Cars,
                Tab::Cars => Tab::Tools,
                Tab::Tools => Tab::Matter,
            };
        }
        let digits = [
            KeyCode::Key1, KeyCode::Key2, KeyCode::Key3, KeyCode::Key4, KeyCode::Key5,
            KeyCode::Key6, KeyCode::Key7, KeyCode::Key8, KeyCode::Key9, KeyCode::Key0,
        ];
        for (i, k) in digits.iter().enumerate() {
            if is_key_pressed(*k) {
                self.tool = Tool::Mat(MATTER[i].0);
                self.tab = Tab::Matter;
            }
        }
    }

    fn handle_mouse(&mut self, view: View, ui: &Ui) {
        let over_ui = ui.mouse.x < PANEL_W || ui.mouse.y < TOP_H;
        let inside = !over_ui && view.rect().contains(ui.mouse);
        let wheel = mouse_wheel().1;
        if inside && wheel != 0.0 {
            let d = if wheel > 0.0 { 1 } else { -1 };
            if matches!(self.tool, Tool::Block(_)) {
                self.rotate_preview(d);
            } else {
                self.brush = (self.brush + d).clamp(0, 40);
            }
        }
        let left = is_mouse_button_down(MouseButton::Left);
        let right = is_mouse_button_down(MouseButton::Right);
        if !inside || (!left && !right) {
            if !left && !right {
                self.last = None;
            }
            return;
        }
        let at = view.to_world(ui.mouse);
        let g = (at.x.floor() as i32, at.y.floor() as i32);
        let from = self.last.map(|p| (p.x.floor() as i32, p.y.floor() as i32)).unwrap_or(g);
        self.last = Some(at);
        let click = is_mouse_button_pressed(MouseButton::Left);
        let rclick = is_mouse_button_pressed(MouseButton::Right);

        if right || (left && self.tool == Tool::Eraser) {
            if click || rclick {
                if let Some(i) = block_at(&self.blocks, at) {
                    self.remove_block(i);
                    return;
                }
            }
            self.world.line(from, g, self.brush.max(2), Mat::Empty);
            return;
        }
        match self.tool {
            Tool::Mat(m) => self.world.line(from, g, self.brush, m),
            Tool::Block(k) if click => self.place_block(k, at),
            Tool::Rotate if click => {
                if let Some(i) = block_at(&self.blocks, at) {
                    self.rotate_block(i);
                }
            }
            Tool::Entry if click => self.entry = Some(g),
            Tool::Exit if click => self.exit = Some(g),
            _ => {}
        }
    }

    // ----- simulation -------------------------------------------------------

    fn update(&mut self, dt: f32) {
        self.time += dt;
        for b in &mut self.booms {
            b.t += dt / 0.7;
        }
        self.booms.retain(|b| b.t < 1.0);
        if let Some((_, t)) = &mut self.toast {
            *t -= dt;
        }
        if self.toast.as_ref().map_or(false, |t| t.1 <= 0.0) {
            self.toast = None;
        }
        if !self.running {
            return;
        }
        for _ in 0..self.speed {
            self.sim_step();
        }
    }

    fn sim_step(&mut self) {
        self.world.step();
        let exit = self.exit;
        for c in &mut self.cars {
            c.step(&self.world, exit);
        }
        if let Some(p) = &mut self.phys {
            p.step(&self.world, &mut self.blocks);
        }
        stamp_blocks(&mut self.world, &self.blocks);

        // Flames heat wooden blocks.
        let touched = std::mem::take(&mut self.world.touched);
        let mut burn: Vec<usize> = vec![];
        for id in touched {
            if let Some(b) = self.blocks.get_mut(id as usize) {
                b.heat = b.heat.saturating_add(5);
                if b.heat >= 150 && !burn.contains(&(id as usize)) {
                    burn.push(id as usize);
                }
            }
        }
        // Explosions shove blocks and wreck nearby cars.
        let blasts = std::mem::take(&mut self.world.blasts);
        for (x, y, r) in blasts {
            let c = vec2(x as f32, y as f32);
            if let Some(p) = &mut self.phys {
                p.blast(&self.blocks, c.x, c.y, r as f32);
            }
            for (i, b) in self.blocks.iter_mut().enumerate() {
                if b.spec().mat == Mat::Wood && (b.pos - c).length() < r as f32 * 1.4 {
                    b.heat = b.heat.saturating_add(80);
                    if b.heat >= 150 && !burn.contains(&i) {
                        burn.push(i);
                    }
                }
            }
            for car in &mut self.cars {
                if (car.p - c).length() < r as f32 * 1.4 && car.fate == Fate::Driving {
                    car.fate = Fate::Exploded;
                }
            }
            self.booms.push(Boom { pos: c, t: 0.0, color: Color::new(1.0, 0.5, 0.1, 1.0) });
        }
        let mut gone: Vec<usize> = (0..self.blocks.len())
            .filter(|&i| {
                let p = self.blocks[i].pos;
                p.y > H as f32 + 60.0 || p.x < -60.0 || p.x > W as f32 + 60.0
            })
            .collect();
        for i in burn {
            if !gone.contains(&i) {
                gone.push(i);
            }
        }
        gone.sort_unstable_by(|a, b| b.cmp(a));
        let mut mask_dirty = false;
        for i in gone {
            let outside = {
                let p = self.blocks[i].pos;
                p.y > H as f32 + 60.0 || p.x < -60.0 || p.x > W as f32 + 60.0
            };
            if outside {
                let mut b = self.blocks.remove(i);
                if let Some(p) = &mut self.phys {
                    p.remove_block(&mut b);
                }
            } else {
                self.burn_block(i);
            }
            mask_dirty = true;
        }
        if mask_dirty {
            self.refresh_mask();
        }

        // Car fates.
        let mut keep = vec![];
        for c in self.cars.drain(..) {
            match c.fate {
                Fate::Driving => keep.push(c),
                Fate::Arrived => {
                    self.arrived += 1;
                    self.booms.push(Boom { pos: c.p, t: 0.0, color: Color::new(0.2, 0.9, 0.4, 1.0) });
                }
                Fate::Dead => self.lost += 1,
                Fate::Exploded => {
                    self.lost += 1;
                    self.world.explode(c.p.x as i32, c.p.y as i32, 16);
                }
            }
        }
        self.cars = keep;
    }

    // ----- drawing ----------------------------------------------------------

    fn draw_scene(&mut self, view: View, renderer: &mut Renderer, ui: &Ui) {
        draw_sky(view, self.time);
        renderer.draw_world(&self.world, view);
        for b in &self.blocks {
            draw_block(b, view);
        }
        if let Some(e) = self.entry {
            draw_flag(view, e, GREEN, false);
        }
        if let Some(x) = self.exit {
            draw_flag(view, x, RED, true);
        }
        for c in &self.cars {
            draw_car(c, view);
        }
        draw_booms(&self.booms, view);

        // Cursor preview.
        let over_ui = ui.mouse.x < PANEL_W || ui.mouse.y < TOP_H;
        if !over_ui && view.rect().contains(ui.mouse) {
            let at = view.to_world(ui.mouse);
            match self.tool {
                Tool::Block(k) => {
                    let cand = self.candidate(k, at);
                    let ok = placement_ok(&self.world, &self.blocks, &cand, None);
                    let tint = if ok { Color::new(0.3, 1.0, 0.5, 1.0) } else { Color::new(1.0, 0.3, 0.3, 1.0) };
                    draw_block_shape(k, view.to_screen(cand.pos), view.s, cand.angle, Some(tint), 0.0);
                }
                Tool::Mat(_) | Tool::Eraser => {
                    let r = (self.brush.max(if self.tool == Tool::Eraser { 2 } else { 0 }) as f32 + 0.5) * view.s;
                    draw_circle_lines(ui.mouse.x, ui.mouse.y, r, 1.5, Color::new(1.0, 1.0, 1.0, 0.9));
                }
                Tool::Rotate => {
                    if let Some(i) = block_at(&self.blocks, at) {
                        let b = &self.blocks[i];
                        let c = view.to_screen(b.pos);
                        draw_circle_lines(c.x, c.y, b.half().length() * view.s, 2.0, ACCENT);
                    }
                }
                _ => {}
            }
        }

        // Mode badge.
        let (label, col) = if self.running { ("SIMULATION", RED) } else { ("CONSTRUCTION", BLUE) };
        let r = view.rect();
        let w = ui.text_w(label, 18) + 38.0;
        rrect(Rect::new(r.x + r.w - w - 10.0, r.y + 10.0, w, 30.0), 15.0, Color::new(0.0, 0.0, 0.0, 0.45));
        draw_circle(r.x + r.w - w + 6.0, r.y + 25.0, 6.0, col);
        ui.text(label, r.x + r.w - w + 18.0, r.y + 31.0, 18, WHITE);
    }

    fn draw_ui(&mut self, ui: &mut Ui, sw: f32, sh: f32) {
        // Panels.
        draw_rectangle(0.0, 0.0, sw, TOP_H, BG);
        draw_rectangle(0.0, TOP_H, PANEL_W, sh - TOP_H, PANEL);
        draw_rectangle(0.0, TOP_H - 2.0, sw, 2.0, Color::new(0.0, 0.0, 0.0, 0.35));

        self.draw_top_bar(ui, sw);
        self.draw_panel(ui, sh);

        if let Some((msg, t)) = &self.toast {
            let a = (*t / 0.5).min(1.0);
            let w = ui.text_w(msg, 22) + 40.0;
            let r = Rect::new(PANEL_W + (sw - PANEL_W - w) / 2.0, sh - 70.0, w, 44.0);
            rrect(r, 22.0, Color::new(0.05, 0.06, 0.1, 0.9 * a));
            ui.text(msg, r.x + 20.0, r.y + 29.0, 22, Color::new(1.0, 1.0, 1.0, a));
        }
    }

    fn draw_top_bar(&mut self, ui: &mut Ui, sw: f32) {
        // (width, id) in a row; shrink to fit narrow windows.
        #[derive(Clone, Copy, PartialEq)]
        enum B {
            Play,
            Reset,
            Car,
            Slower,
            Faster,
            Rotate,
            Slot(usize),
            Save,
            Load,
            World,
            Blank,
            Full,
        }
        let items: [(f32, B); 14] = [
            (124.0, B::Play),
            (112.0, B::Reset),
            (120.0, B::Car),
            (42.0, B::Slower),
            (54.0, B::Faster),
            (122.0, B::Rotate),
            (38.0, B::Slot(0)),
            (38.0, B::Slot(1)),
            (38.0, B::Slot(2)),
            (110.0, B::Save),
            (118.0, B::Load),
            (104.0, B::World),
            (92.0, B::Blank),
            (84.0, B::Full),
        ];
        let gap = 8.0;
        let total: f32 = items.iter().map(|i| i.0).sum::<f32>() + gap * (items.len() as f32 + 1.0) + 24.0;
        let k = (sw / total).min(1.0);
        let mut x = 12.0 * k;
        let h = 48.0;
        let y = (TOP_H - h) / 2.0 - 1.0;
        for (w, b) in items {
            let w = w * k;
            let r = Rect::new(x, y, w, h);
            x += w + gap * k;
            if matches!(b, B::Slower) {
                x -= gap * k * 0.5;
            }
            match b {
                B::Play => {
                    let (label, icon) = if self.running { ("Pause", Icon::Pause) } else { ("Jouer", Icon::Play) };
                    if ui.button(r, label, icon, !self.running, true, GREEN) || (self.running && false) {
                        let run = !self.running;
                        self.set_running(run);
                    }
                }
                B::Reset => {
                    if ui.button(r, "Retour", Icon::Reset, false, true, ACCENT) {
                        self.reset();
                    }
                }
                B::Car => {
                    if ui.button(r, "Voiture", Icon::Car, false, true, ACCENT) {
                        self.launch_car();
                    }
                }
                B::Slower => {
                    if ui.button(r, "", Icon::Minus, false, self.speed > 1, ACCENT) {
                        self.speed -= 1;
                    }
                }
                B::Faster => {
                    let label = format!("x{}", self.speed);
                    let minus_w = 0.0;
                    let _ = minus_w;
                    if ui.button(r, "", Icon::Plus, false, self.speed < 6, ACCENT) {
                        self.speed += 1;
                    }
                    ui.text_center(&label, r.x + r.w + 14.0 * k, r.y + h / 2.0 + 6.0, 18, DIM);
                    x += 30.0 * k;
                }
                B::Rotate => {
                    if ui.button(r, "Tourner", Icon::Rotate, matches!(self.tool, Tool::Block(_)) && self.rot90 != 0, true, BLUE) {
                        self.rotate_preview(1);
                    }
                }
                B::Slot(i) => {
                    let used = save::slot_used(i);
                    let label = format!("{}", i + 1);
                    if ui.button(r, &label, Icon::None, self.slot == i, true, if used { ACCENT } else { DIM }) {
                        self.slot = i;
                    }
                    if used && self.slot != i {
                        draw_circle(r.x + r.w - 7.0, r.y + 7.0, 4.0, ACCENT);
                    }
                }
                B::Save => {
                    if ui.button(r, "Sauver", Icon::Save, false, true, ACCENT) {
                        self.save();
                    }
                }
                B::Load => {
                    if ui.button(r, "Charger", Icon::Load, false, save::slot_used(self.slot), ACCENT) {
                        self.load();
                    }
                }
                B::World => {
                    if ui.button(r, "Monde", Icon::World, false, true, ACCENT) {
                        self.new_world();
                    }
                }
                B::Blank => {
                    if ui.button(r, "Vide", Icon::Blank, false, true, ACCENT) {
                        self.empty_world();
                    }
                }
                B::Full => {
                    if ui.button(r, "Plein", Icon::Full, self.fullscreen, true, BLUE) {
                        self.toggle_fullscreen();
                    }
                }
            }
        }
    }

    fn draw_panel(&mut self, ui: &mut Ui, sh: f32) {
        let tabs = [(Tab::Matter, "Matières"), (Tab::Blocks, "Blocs"), (Tab::Cars, "Voitures"), (Tab::Tools, "Outils")];
        let tw = (PANEL_W - 8.0 * 3.0) / 2.0;
        for (i, (tab, label)) in tabs.iter().enumerate() {
            let r = Rect::new(8.0 + (i % 2) as f32 * (tw + 8.0), TOP_H + 8.0 + (i / 2) as f32 * 40.0, tw, 34.0);
            if ui.button(r, label, Icon::None, self.tab == *tab, true, ACCENT) {
                self.tab = *tab;
            }
        }
        let top = TOP_H + 8.0 + 80.0 + 8.0;
        let tile_w = tw;
        match self.tab {
            Tab::Matter => {
                for (i, (m, label)) in MATTER.iter().enumerate() {
                    let r = Rect::new(8.0 + (i % 2) as f32 * (tile_w + 8.0), top + (i / 2) as f32 * 78.0, tile_w, 70.0);
                    let col = mat_rgb(*m);
                    if ui.tile(r, label, self.tool == Tool::Mat(*m), |s| {
                        rrect(s, 8.0, col);
                        rrect(Rect::new(s.x, s.y, s.w, s.h * 0.4), 8.0, Color::new(1.0, 1.0, 1.0, 0.18));
                        if matches!(m, Mat::Fire | Mat::Lava) {
                            draw_circle(s.x + s.w * 0.5, s.y + s.h * 0.5, s.h * 0.35, Color::new(1.0, 0.9, 0.3, 0.55));
                        }
                    }) {
                        self.tool = Tool::Mat(*m);
                    }
                }
            }
            Tab::Blocks => {
                for (i, k) in BlockKind::ALL.iter().enumerate() {
                    let r = Rect::new(8.0 + (i % 2) as f32 * (tile_w + 8.0), top + (i / 2) as f32 * 78.0, tile_w, 70.0);
                    let name = k.spec().name;
                    let spec = k.spec();
                    let angle = self.rot90 as f32 * std::f32::consts::FRAC_PI_2;
                    let scale = (r.w - 24.0).min(30.0 * 2.0) / spec.w.max(spec.h).max(30.0) * if spec.w > 60.0 { 1.0 } else { 1.0 };
                    if ui.tile(r, name, self.tool == Tool::Block(*k), |s| {
                        let sc = (s.w / spec.w.max(spec.h)).min(1.6).min(s.h / spec.w.min(spec.h) / 1.0).max(0.4);
                        let sc = sc.min(scale.max(0.5) * 2.2);
                        draw_block_shape(*k, vec2(s.x + s.w / 2.0, s.y + s.h / 2.0), sc, angle, None, 0.0);
                    }) {
                        self.tool = Tool::Block(*k);
                    }
                }
                ui.text("Molette ou Q/E : tourner", 10.0, top + 3.0 * 78.0 + 14.0, 15, DIM);
                ui.text("Maj : poser sans grille", 10.0, top + 3.0 * 78.0 + 34.0, 15, DIM);
            }
            Tab::Cars => {
                for (i, k) in ModelKind::ALL.iter().enumerate() {
                    let m = k.model();
                    let r = Rect::new(8.0, top + i as f32 * 82.0, PANEL_W - 16.0, 74.0);
                    let sel = self.model == *k;
                    let hot = ui.hover(r);
                    rrect(r, 12.0, if hot { ui::TILE_HOT } else { ui::TILE });
                    if sel {
                        ui::rrect_lines(r, 12.0, 3.5, ACCENT);
                    }
                    let c = Color::from_rgba(m.color.0, m.color.1, m.color.2, 255);
                    rrect(Rect::new(r.x + 10.0, r.y + 30.0, 52.0, 16.0), 5.0, c);
                    rrect(Rect::new(r.x + 20.0, r.y + 18.0, 28.0, 16.0), 5.0, c);
                    draw_circle(r.x + 22.0, r.y + 48.0, 8.0, Color::from_rgba(26, 26, 32, 255));
                    draw_circle(r.x + 50.0, r.y + 48.0, 8.0, Color::from_rgba(26, 26, 32, 255));
                    ui.text(m.name, r.x + 74.0, r.y + 22.0, 20, if sel { ACCENT } else { TEXT });
                    ui.text(m.blurb, r.x + 74.0, r.y + 40.0, 14, DIM);
                    let bars = [(m.top_speed / 2.1, BLUE), (m.torque / 1.2, GREEN), (m.crash_speed / 5.0, ACCENT)];
                    for (j, (v, col)) in bars.iter().enumerate() {
                        let bx = r.x + 74.0;
                        let by = r.y + 49.0 + j as f32 * 7.0 - 2.0;
                        draw_rectangle(bx, by, 140.0, 4.0, Color::new(0.0, 0.0, 0.0, 0.35));
                        draw_rectangle(bx, by, 140.0 * v.clamp(0.05, 1.0), 4.0, *col);
                    }
                    if hot && ui.pressed {
                        self.model = *k;
                    }
                }
                ui.text("Bleu vitesse · vert couple · orange solidité", 10.0, top + 4.0 * 82.0 + 10.0, 13, DIM);
            }
            Tab::Tools => {
                let tools: [(Tool, &str, Icon); 4] = [
                    (Tool::Eraser, "Gomme", Icon::Eraser),
                    (Tool::Rotate, "Tourner", Icon::Rotate),
                    (Tool::Entry, "Entrée", Icon::Flag),
                    (Tool::Exit, "Sortie", Icon::Flag),
                ];
                for (i, (t, label, icon)) in tools.iter().enumerate() {
                    let r = Rect::new(8.0 + (i % 2) as f32 * (tile_w + 8.0), top + (i / 2) as f32 * 78.0, tile_w, 70.0);
                    let flag_col = if *t == Tool::Exit { RED } else if *t == Tool::Entry { GREEN } else { TEXT };
                    if ui.tile(r, label, self.tool == *t, |s| {
                        draw_icon(*icon, vec2(s.x + s.w / 2.0, s.y + s.h / 2.0), 34.0, flag_col);
                    }) {
                        self.tool = *t;
                    }
                }
                ui.text("Tourner : clique un bloc posé", 10.0, top + 2.0 * 78.0 + 14.0, 14, DIM);
                ui.text("Clic droit : gomme partout", 10.0, top + 2.0 * 78.0 + 32.0, 14, DIM);
            }
        }

        // Bottom: brush size, stats, help.
        let by = sh - 150.0;
        ui.text("Pinceau", 12.0, by + 6.0, 18, DIM);
        let minus = Rect::new(12.0, by + 14.0, 40.0, 34.0);
        let plus = Rect::new(PANEL_W - 52.0, by + 14.0, 40.0, 34.0);
        if ui.button(minus, "", Icon::Minus, false, self.brush > 0, ACCENT) {
            self.brush -= 1;
        }
        if ui.button(plus, "", Icon::Plus, false, self.brush < 40, ACCENT) {
            self.brush += 1;
        }
        let bar = Rect::new(60.0, by + 22.0, PANEL_W - 120.0, 18.0);
        rrect(bar, 9.0, Color::new(0.0, 0.0, 0.0, 0.35));
        let fill = (self.brush as f32 + 1.0) / 41.0;
        rrect(Rect::new(bar.x, bar.y, (bar.w * fill).max(18.0), bar.h), 9.0, ACCENT);
        ui.text_center(&format!("{}", self.brush), bar.x + bar.w / 2.0, bar.y + 14.0, 16, BG);

        ui.text(&format!("Arrivées {}   Perdues {}", self.arrived, self.lost), 12.0, by + 74.0, 18, TEXT);
        ui.text("Espace jouer · R retour · C voiture", 12.0, by + 98.0, 13, DIM);
        ui.text("F11 plein écran · F5/F9 sauver/charger", 12.0, by + 116.0, 13, DIM);
        ui.text("Clic droit gomme · Tab changer d'onglet", 12.0, by + 134.0, 13, DIM);
    }
}

fn ground_y(world: &World, x: i32) -> i32 {
    (0..H as i32).find(|&y| world.raw(x, y).is_hard()).unwrap_or(H as i32 - 1)
}

/// Builds a little scene for `--demo`: a bridge over a pit, a tower, a wheel.
fn demo_scene(g: &mut Game) {
    g.seed = 2;
    let (e, x) = g.world.generate(2);
    g.entry = Some(e);
    g.exit = Some(x);
    g.fresh();
    let gx = 300;
    let gy = ground_y(&g.world, gx);
    let mut z = |kind: BlockKind, x: f32, y: f32| {
        g.blocks.push(Block::new(kind, vec2(x, y), 0.0));
    };
    z(BlockKind::Brick, gx as f32, gy as f32 - 6.0);
    z(BlockKind::Brick, gx as f32, gy as f32 - 18.0);
    z(BlockKind::Long, gx as f32 + 6.0, gy as f32 - 30.0);
    z(BlockKind::Cube, gx as f32 + 60.0, gy as f32 - 12.0);
    z(BlockKind::Wheel, gx as f32 + 120.0, gy as f32 - 80.0);
    z(BlockKind::Plank, gx as f32 + 160.0, gy as f32 - 40.0);
    g.world.paint(430, gy - 40, 8, Mat::Lava);
    g.world.paint(200, gy - 60, 10, Mat::Water);
    g.world.paint(520, gy - 20, 5, Mat::Fire);
    g.refresh_mask();
    g.save();
    g.load();
    g.tab = Tab::Blocks;
    g.tool = Tool::Block(BlockKind::Brick);
    g.model = ModelKind::Offroad;
    g.launch_car();
}

#[macroquad::main(window_conf)]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let shot = args.iter().position(|a| a == "--demo").and_then(|i| args.get(i + 1)).cloned();

    let font = load_ttf_font_from_bytes(include_bytes!("../assets/font.ttf")).expect("font");
    let mut ui = Ui { font, mouse: Vec2::ZERO, pressed: false };
    let mut renderer = Renderer::new();
    let mut game = Game::new();
    if shot.is_some() {
        demo_scene(&mut game);
        let tab = args.iter().position(|a| a == "--tab").and_then(|i| args.get(i + 1));
        game.tab = match tab.map(|s| s.as_str()) {
            Some("matter") => Tab::Matter,
            Some("cars") => Tab::Cars,
            Some("tools") => Tab::Tools,
            _ => Tab::Blocks,
        };
    }
    let mut frame = 0u32;

    loop {
        let (sw, sh) = (screen_width(), screen_height());
        let view = View::fit(Rect::new(PANEL_W, TOP_H, sw - PANEL_W, sh - TOP_H));
        ui.begin();
        game.handle_keys();
        game.handle_mouse(view, &ui);
        game.update(get_frame_time().min(0.05));

        clear_background(BG);
        game.draw_scene(view, &mut renderer, &ui);
        game.draw_ui(&mut ui, sw, sh);

        frame += 1;
        if let Some(path) = &shot {
            if frame == 360 {
                get_screen_data().export_png(path);
                std::process::exit(0);
            }
        }
        next_frame().await;
    }
}
