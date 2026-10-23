mod backdrop;
mod blocks;
mod camera;
mod car;
mod fireworks;
mod firetruck;
mod debris;
mod mech;
mod fluidgame;
mod lens;
mod mechgame;
mod pipes;
mod plane;
mod render;
mod save;
mod select;
mod sensors;
mod shovel;
mod science;
mod sound;
mod ui;
mod whale;
mod world;

use blocks::{anchor_world, block_at, clip, node_at, placement_ok, snap, stamp_all, Anchor, Block, BlockKind, Cable, CableMat, Member, MemberMat, Physics, Spring, SpringKind, MEMBER_MAX_LEN, SPRING_MAX_LEN};
use car::{collide_cars, Car, Fate, ModelKind, Tuning};
use macroquad::prelude::*;
use render::{RenderMode, cable_preview, draw_block, draw_block_shape, draw_booms, draw_cable, draw_car, draw_flag, draw_member, draw_spring, draw_spring_between, Boom, Renderer, View};
use science::{info, mat_color, MATTERS, RECIPES};
use mech::{GearSize, Mechanism};
use mechgame::MechDraft;
use sound::{Ev, Mix, Sounds};
use ui::{draw_icon, rrect, Drawer, Icon, Ui, ACCENT, BG, BLUE, DIM, GREEN, PANEL, RED, TEXT, TILE, TILE_HOT};
use world::{Biome, Cell, Mat, PaintRule, Season, World, H, W};

/// Simulation steps per second at speed x1 (the rigid bodies use the same 1/60 s step).
const STEPS_PER_SECOND: f32 = 60.0;
const PANEL_W: f32 = 244.0;
/// No top bar any more: the world starts at the very top.
const TOP_H: f32 = 0.0;
/// Width of the sliding views palette on the right, and of its always-visible tab.
const VIEWS_W: f32 = 176.0;
const VIEWS_TAB: f32 = 22.0;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Matter,
    Blocks,
    Cars,
    Mech,
    Fluid,
    Planet,
}

#[derive(Clone, Copy, PartialEq)]
enum ViewMode {
    Normal,
    Thermal,
    Stress,
    Wind,
    Pressure,
    Humidity,
}

#[derive(Clone, Copy, PartialEq)]
enum PaintMode {
    Add,
    Replace,
    Only,
}

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Mat(Mat),
    Block(BlockKind),
    Cable(CableMat),
    Spring(SpringKind),
    Beam(MemberMat),
    Gear(GearSize),
    Motor,
    Axle,
    Rail,
    Crank,
    Pivot,
    Screw,
    Winch,
    Pulley,
    Pipe,
    Valve,
    Pump,
    Shovel,
    Select,
    Eraser,
    Hammer,
    Loupe,
    Sensor,
    Entry,
    Exit,
}


fn window_conf() -> Conf {
    Conf {
        window_title: "AWorld".to_owned(),
        window_width: (PANEL_W + 1280.0) as i32,
        window_height: (TOP_H + 768.0) as i32,
        window_resizable: true,
        sample_count: 4,
        ..Default::default()
    }
}

/// Everything needed to put the world back as it was just before a brick was dropped.
struct Snapshot {
    state: world::State,
    blocks: Vec<Block>,
    cables: Vec<Cable>,
    members: Vec<Member>,
    springs: Vec<Spring>,
    mech: Mechanism,
    cars: Vec<Car>,
    arrived: u32,
    lost: u32,
}

/// A spring being set up: first end fixed, rest length chosen, far end still to place.
struct SpringDraft {
    a: Anchor,
    pa: Vec2,
    /// Where the rest length was set (the second click) and that length.
    rest: Option<(Vec2, f32)>,
}

struct Game {
    science_fresh: bool,
    sounds: Sounds,
    /// Fire, lava and embers on screen (for the crackle), refreshed twice a second.
    heat_cells: u32,
    undo: Vec<Snapshot>,
    world: World,
    snap_cells: Vec<Cell>,
    snap_blocks: Vec<Block>,
    snap_cables: Vec<Cable>,
    snap_members: Vec<Member>,
    snap_springs: Vec<Spring>,
    mech: Mechanism,
    snap_mech: Mechanism,
    mech_draft: Option<MechDraft>,
    /// How the next motor turns, and whether it reverses itself when overloaded.
    motor_dir: f32,
    motor_reverse: bool,
    /// The motor shown in the remote control under the views (a gear id).
    selected_motor: Option<u32>,
    blocks: Vec<Block>,
    cables: Vec<Cable>,
    members: Vec<Member>,
    springs: Vec<Spring>,
    spring_draft: Option<SpringDraft>,
    cable_from: Option<Anchor>,
    /// The rare plane that drifts across the sky, for fun.
    planes: plane::Planes,
    /// Whales blowing in wide seas, the fire engine called by a big fire, and the fireworks that
    /// powder going off at night sends up.
    whales: whale::Whales,
    trucks: firetruck::FireTrucks,
    fireworks: fireworks::Fireworks,
    /// The night (day number) whose fireworks were explained already.
    fireworks_told: u32,
    /// Where the tool strip, the corner buttons are (so that clicks on them do not paint the world).
    tools_hit: Rect,
    corner_hit: Rect,
    hud_hit: Rect,
    launch_hit: Rect,
    /// The left palette hides like the views on the right; pinned, it stays out (and the picture
    /// beside it). `panel_hit` is where it is, so that clicks on it do not paint the world.
    panel: Drawer,
    panel_hit: Rect,
    /// The car settings are shown under the views.
    car_remote: bool,
    /// The selection tool: what is selected and the drag in progress.
    selection: Option<select::Sel>,
    drag: Option<select::Drag>,
    /// The floating player: where the mouse was, for how long it has stood still, and how far out it is.
    last_mouse: Vec2,
    idle: f32,
    player_alpha: f32,
    player_pin: bool,
    /// The first end of the pipe being drawn.
    pipe_from: Option<Vec2>,
    /// Turns a gear needs to open a valve from shut, for the next link.
    valve_turns: f32,
    /// What the shovel carries.
    shovel: Option<shovel::Load>,
    /// Seconds since the shovel was filled (the bubble fills over a moment).
    shovel_age: f32,
    /// Seconds the shovel's button has been held (the part it will let go of grows with it).
    shovel_charge: Option<f32>,
    /// Thermometer, barometer and hygrometer probes with their curves.
    sensors: sensors::Sensors,
    /// The "leave the game?" window is open (the simulation is paused while it is), and whether the
    /// simulation was running before it opened.
    quit_modal: bool,
    quit_resume: bool,
    /// Which button of the leave window is selected: false = go back to the game, true = leave.
    quit_choice: bool,
    /// Seconds during which a simplified simulation is not restored, and how long the frames have
    /// been fast enough to restore it.
    lod_lock: f32,
    fast_frames: u32,
    /// How far the science page is scrolled, in pixels.
    science_scroll: f32,
    /// Pulleys the cable being drawn has been threaded over so far.
    cable_via: Vec<u32>,
    /// Cable elasticity setting (1 = as the material is, higher = stretchier).
    cable_soft: f32,
    member_from: Option<(Anchor, Vec2)>,
    phys: Option<Physics>,
    cars: Vec<Car>,
    tunings: [Tuning; 4],
    selected_car: Option<u32>,
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
    /// Pieces of exploded cars, flying and bouncing.
    debris: debris::Debris,
    hammer_fx: Vec<(Vec2, f32)>,
    time: f32,
    fullscreen: bool,
    mode: ViewMode,
    /// How the brush treats existing material, and the material "only" mode targets.
    paint_mode: PaintMode,
    only_target: Mat,
    /// The state each matter of the palette paints (an index into its states), and the matter
    /// whose states are shown under its button (the little arrow).
    matter_state: [usize; MATTERS.len()],
    matter_menu: Option<usize>,
    /// The views on the right edge, a side menu like the palette.
    views: Drawer,
    views_hit: Rect,
    /// Ctrl + wheel / Ctrl + drag zoom and move the view; Escape brings back the whole world.
    camera: camera::Camera,
    /// Camera shake in pixels (explosions), and frame-rate watching for auto speed.
    shake: f32,
    frame_ema: f32,
    slow_frames: u32,
    /// Simulation steps owed to the clock: the world runs 60 steps a second (times the speed)
    /// whatever the screen's refresh rate.
    sim_acc: f32,
    beam_broke: bool,
    broken_members: Vec<u32>,
    /// Building does not use up the world's materials: one choice for the editing
    /// phase and one for the simulation (picked while editing).
    unlimited: bool,
    unlimited_sim: bool,
    census: [u32; world::NMAT],
    show_science: bool,
    hover_info: Option<(String, String)>,
}

impl Game {
    fn new() -> Self {
        let mut world = World::new();
        let (entry, exit) = world.generate_biome(1, Biome::Countryside);
        Game {
            science_fresh: false,
            sounds: Sounds::silent(),
            heat_cells: 0,
            undo: vec![],
            snap_cells: world.snapshot(),
            snap_blocks: vec![],
            snap_cables: vec![],
            snap_members: vec![],
            snap_springs: vec![],
            mech: Mechanism::default(),
            snap_mech: Mechanism::default(),
            mech_draft: None,
            motor_dir: 1.0,
            motor_reverse: false,
            selected_motor: None,
            world,
            blocks: vec![],
            cables: vec![],
            members: vec![],
            springs: vec![],
            spring_draft: None,
            cable_from: None,
            planes: plane::Planes::new(),
            whales: whale::Whales::new(),
            trucks: firetruck::FireTrucks::new(),
            fireworks: fireworks::Fireworks::new(),
            fireworks_told: 0,
            tools_hit: Rect::new(-10.0, -10.0, 0.0, 0.0),
            corner_hit: Rect::new(-10.0, -10.0, 0.0, 0.0),
            hud_hit: Rect::new(-10.0, -10.0, 0.0, 0.0),
            launch_hit: Rect::new(-10.0, -10.0, 0.0, 0.0),
            panel: Drawer { open: 1.0, pin: false },
            panel_hit: Rect::new(0.0, 0.0, PANEL_W, 10000.0),
            car_remote: false,
            selection: None,
            drag: None,
            last_mouse: Vec2::ZERO,
            idle: 0.0,
            player_alpha: 1.0,
            player_pin: false,
            pipe_from: None,
            valve_turns: 3.0,
            shovel: None,
            shovel_age: 0.0,
            shovel_charge: None,
            sensors: sensors::Sensors::default(),
            quit_modal: false,
            quit_resume: false,
            quit_choice: false,
            lod_lock: 0.0,
            fast_frames: 0,
            science_scroll: 0.0,
            cable_via: vec![],
            cable_soft: 1.0,
            member_from: None,
            phys: None,
            cars: vec![],
            tunings: [Tuning::default(); 4],
            selected_car: None,
            entry: Some(entry),
            exit: Some(exit),
            running: false,
            tool: Tool::Mat(Mat::Sand),
            tab: Tab::Matter,
            brush: 6,
            rot90: 0,
            speed: 3,
            model: ModelKind::Buggy,
            arrived: 0,
            lost: 0,
            seed: 1,
            slot: 0,
            last: None,
            toast: None,
            booms: vec![],
            debris: debris::Debris::default(),
            hammer_fx: vec![],
            time: 0.0,
            fullscreen: false,
            mode: ViewMode::Normal,
            paint_mode: PaintMode::Add,
            only_target: Mat::Empty,
            matter_state: MATTERS.map(|m| m.default),
            matter_menu: None,
            views: Drawer::new(false),
            views_hit: Rect::new(-10.0, -10.0, 0.0, 0.0),
            camera: camera::Camera::default(),
            shake: 0.0,
            frame_ema: 0.016,
            slow_frames: 0,
            sim_acc: 0.0,
            beam_broke: false,
            broken_members: vec![],
            unlimited: true,
            unlimited_sim: false,
            census: [0; world::NMAT],
            show_science: false,
            hover_info: None,
        }
    }

    /// Remember the world right now so the next edit can be undone.
    fn checkpoint(&mut self) {
        self.undo.push(Snapshot {
            state: self.world.capture(),
            blocks: self.blocks.iter().map(|b| Block { body: None, ..b.clone() }).collect(),
            cables: self.cables.iter().map(Cable::fresh).collect(),
            members: self.members.iter().map(Member::fresh).collect(),
            springs: self.springs.iter().map(Spring::fresh).collect(),
            mech: self.mech.clone(),
            cars: self.cars.clone(),
            arrived: self.arrived,
            lost: self.lost,
        });
        if self.undo.len() > 15 {
            self.undo.remove(0);
        }
    }

    fn undo(&mut self) {
        let Some(s) = self.undo.pop() else {
            self.say("Rien à annuler");
            return;
        };
        self.world.restore_state(&s.state);
        self.blocks = s.blocks;
        self.cables = s.cables;
        self.members = s.members;
        self.springs = s.springs;
        self.mech = s.mech;
        self.mech_draft = None;
        self.spring_draft = None;
        self.cars = s.cars;
        self.arrived = s.arrived;
        self.lost = s.lost;
        self.cable_from = None;
        self.cable_via.clear();
        self.booms.clear();
        self.debris.clear();
        self.phys = None;
        if self.running {
            self.build_physics();
        }
        self.refresh_mask();
        let left = self.undo.len();
        self.say(format!("Annulé ({left} restant{})", if left > 1 { "s" } else { "" }));
    }

    /// Placeholder: breaking glass is not an undoable edit.
    fn checkpoint_quiet(&mut self) {}

    /// Is building free right now? Depends on the phase we are in.
    fn free_now(&self) -> bool {
        if self.running { self.unlimited_sim } else { self.unlimited }
    }

    fn say(&mut self, s: impl Into<String>) {
        self.toast = Some((s.into(), 2.5));
    }

    fn refresh_mask(&mut self) {
        stamp_all(&mut self.world, &self.blocks, &self.members);
    }

    fn model_index(&self) -> usize {
        ModelKind::ALL.iter().position(|k| *k == self.model).unwrap_or(0)
    }

    // ----- lifecycle --------------------------------------------------------

    fn build_physics(&mut self) {
        let mut p = Physics::build(&mut self.blocks);
        for m in &mut self.members {
            *m = m.fresh();
        }
        p.build_members(&self.blocks, &mut self.members);
        self.mech_prune_wheels();
        for g in &mut self.mech.gears {
            g.arm_base = None;
        }
        for pv in &self.mech.pivots {
            p.add_pivot(&self.blocks, &self.members, pv.wheel, &pv.support);
        }
        for rod in &self.mech.rods {
            if rod.broken {
                continue;
            }
            let (mech_end, node) = match (rod.a, rod.b) {
                (e, mech::End::Node(n)) if !matches!(e, mech::End::Node(_)) => (e, n),
                (mech::End::Node(n), e) if !matches!(e, mech::End::Node(_)) => (e, n),
                _ => continue,
            };
            if let Some(pin) = self.mech.end_point(&mech_end) {
                p.add_pin_rod(&self.blocks, &self.members, rod.uid, pin, &node, rod.len);
            }
        }
        self.fluid_prepare();
        self.mech_physics_setup(&mut p);
        for i in 0..self.cables.len() {
            self.cables[i].reset_runtime();
            self.mech_prepare_cable(i);
            p.add_cable(&self.blocks, &self.members, &mut self.cables[i]);
        }
        for sp in &mut self.springs {
            *sp = sp.fresh();
            p.add_spring(&self.blocks, &self.members, sp);
        }
        self.phys = Some(p);
    }

    fn set_running(&mut self, run: bool) {
        if run && !self.running {
            self.snap_cells = self.world.snapshot();
            self.snap_blocks = self.blocks.iter().map(|b| Block { body: None, ..b.clone() }).collect();
            self.snap_cables = self.cables.iter().map(Cable::fresh).collect();
            self.snap_members = self.members.iter().map(Member::fresh).collect();
            self.snap_springs = self.springs.iter().map(Spring::fresh).collect();
            self.snap_mech = self.mech.clone();
            if self.phys.is_none() {
                self.build_physics();
            }
        }
        self.running = run;
    }

    fn reset(&mut self) {
        self.world.restore(&self.snap_cells);
        self.sensors.clear_history(&self.world);
        self.blocks = self.snap_blocks.clone();
        self.cables = self.snap_cables.iter().map(Cable::fresh).collect();
        self.members = self.snap_members.iter().map(Member::fresh).collect();
        self.springs = self.snap_springs.iter().map(Spring::fresh).collect();
        self.mech = self.snap_mech.clone();
        self.mech_draft = None;
        self.spring_draft = None;
        self.cable_from = None;
        self.cable_via.clear();
        self.member_from = None;
        self.phys = None;
        self.cars.clear();
        self.debris.clear();
        self.running = false;
        self.refresh_mask();
    }

    fn fresh(&mut self) {
        self.undo.clear();
        self.blocks.clear();
        self.cables.clear();
        self.members.clear();
        self.springs.clear();
        self.mech = Mechanism::default();
        self.snap_mech = Mechanism::default();
        self.mech_draft = None;
        self.spring_draft = None;
        self.cable_from = None;
        self.cable_via.clear();
        self.member_from = None;
        self.phys = None;
        self.cars.clear();
        self.debris.clear();
        self.selected_car = None;
        self.running = false;
        self.arrived = 0;
        self.lost = 0;
        self.refresh_mask();
        self.snap_cells = self.world.snapshot();
        self.snap_blocks.clear();
        self.snap_cables.clear();
        self.snap_members.clear();
        self.snap_springs.clear();
    }

    fn new_world(&mut self, biome: Biome) {
        self.seed += 1;
        let (e, x) = self.world.generate_biome(self.seed, biome);
        self.entry = Some(e);
        self.exit = Some(x);
        self.fresh();
        self.say(format!("Nouveau monde : {}", biome.name()));
    }

    fn empty_world(&mut self) {
        self.world.clear();
        self.entry = None;
        self.exit = None;
        self.fresh();
    }

    fn launch_car(&mut self) {
        let Some(e) = self.entry else {
            self.say("Pose d'abord une Entrée (onglet Voitures)");
            return;
        };
        let crowded = self.cars.iter().any(|c| (c.p - vec2(e.0 as f32, e.1 as f32)).length() < 45.0);
        if crowded {
            self.say("Laisse la voiture précédente partir");
            return;
        }
        if self.cars.len() < 8 {
            let car = Car::with_tuning(self.model, e, self.tunings[self.model_index()]);
            self.selected_car = Some(car.id);
            self.cars.push(car);
        }
        self.set_running(true);
    }

    fn save(&mut self) {
        match save::save_slot(self.slot, &self.world, self.entry, self.exit, &self.blocks, &self.cables, &self.members, &self.springs, &self.mech) {
            Ok(()) => self.say(format!("Monde sauvegardé (emplacement {})", self.slot + 1)),
            Err(e) => self.say(format!("Échec de la sauvegarde : {e}")),
        }
    }

    fn load(&mut self) {
        match save::load_slot(self.slot) {
            Ok(l) => {
                self.world.clear();
                self.world.biome = l.biome;
                self.world.clock = l.clock;
                self.world.reset_temps();
                self.world.load_mats(&l.mats);
                self.entry = l.entry;
                self.exit = l.exit;
                self.fresh();
                self.blocks = l.blocks;
                self.cables = l.cables;
                self.members = l.members;
                self.springs = l.springs;
                self.mech = l.mech;
                {
                    let (blocks, members) = (&self.blocks, &self.members);
                    self.mech.follow_mounts(|a| anchor_world(a, blocks, members));
                }
                self.refresh_reel_anchors();
                self.snap_mech = self.mech.clone();
                self.snap_blocks = self.blocks.clone();
                self.snap_cables = self.cables.iter().map(Cable::fresh).collect();
                self.snap_members = self.members.iter().map(Member::fresh).collect();
                self.snap_springs = self.springs.iter().map(Spring::fresh).collect();
                self.refresh_mask();
                self.say(format!("Monde chargé (emplacement {})", self.slot + 1));
            }
            Err(e) => self.say(format!("Impossible de charger : {e}")),
        }
    }

    /// Esc backs out of whatever is going on, one level at a time: the leave-the-game window, the
    /// science page, a build in progress, full screen, and finally the window that asks to leave.
    fn on_escape(&mut self) {
        let drafting = self.cable_from.is_some() || self.member_from.is_some() || self.spring_draft.is_some() || self.mech_draft.is_some() || self.pipe_from.is_some();
        if self.quit_modal {
            self.close_quit_modal();
        } else if self.show_science {
            self.show_science = false;
        } else if self.selection.is_some() {
            self.selection = None;
            self.drag = None;
        } else if drafting {
            self.cable_from = None;
            self.cable_via.clear();
            self.member_from = None;
            self.spring_draft = None;
            self.mech_draft = None;
            self.pipe_from = None;
        } else if !self.camera.is_standard() {
            self.camera.reset();
            self.say("Vue standard");
        } else if self.fullscreen {
            self.toggle_fullscreen();
        } else {
            self.quit_resume = self.running;
            self.quit_modal = true;
            self.quit_choice = false;
            self.set_running(false);
        }
    }

    /// Enter in the leave-the-game window validates the selected button: true when the game must
    /// close, false when it goes back to the game (or the window is not open).
    fn on_enter_in_quit_modal(&mut self) -> bool {
        if self.quit_modal && !self.quit_choice {
            self.close_quit_modal();
            return false;
        }
        self.quit_modal
    }

    fn close_quit_modal(&mut self) {
        self.quit_modal = false;
        if self.quit_resume {
            self.set_running(true);
        }
    }

    fn quit_game() {
        #[cfg(not(target_arch = "wasm32"))]
        std::process::exit(0);
        // A page cannot close itself: the web version just goes back to the game.
        #[cfg(target_arch = "wasm32")]
        macroquad::miniquad::window::request_quit();
    }

    fn toggle_fullscreen(&mut self) {
        self.fullscreen = !self.fullscreen;
        set_fullscreen(self.fullscreen);
    }

    // ----- block and cable editing ------------------------------------------

    fn candidate(&self, kind: BlockKind, at: Vec2) -> Block {
        let angle = self.rot90 as f32 * std::f32::consts::FRAC_PI_2;
        let pos = if is_key_down(KeyCode::LeftShift) { at } else { clip(&self.blocks, kind, angle, snap(kind, self.rot90, at)) };
        Block::new(kind, pos, angle)
    }

    fn place_block(&mut self, kind: BlockKind, at: Vec2) {
        let mut b = self.candidate(kind, at);
        if !placement_ok(&self.world, &self.blocks, &b, None) {
            self.say("Pas de place ici : les blocs ne peuvent pas se superposer");
            return;
        }
        let (from, units) = kind.cost();
        let free = self.free_now();
        if !free {
            self.census = self.world.census();
            let have: u32 = from.iter().map(|m| self.census[*m as usize]).sum();
            if have < units {
                self.say(format!("Pas assez de {} : il en faut {units}, il y en a {have}. Peins-en !", info(from[0]).name.to_lowercase()));
                return;
            }
        }
        self.checkpoint();
        if !free {
            self.world.consume(from, units, (at.x as i32, at.y as i32));
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

    fn remove_cables_of(&mut self, uid: u32) {
        let mut keep = vec![];
        for mut c in self.cables.drain(..) {
            if c.attached_to(uid) {
                if let Some(p) = &mut self.phys {
                    p.remove_cable(&mut c);
                }
            } else {
                keep.push(c);
            }
        }
        self.cables = keep;
    }

    fn remove_springs_where(&mut self, test: impl Fn(&Spring) -> bool) {
        let mut keep = vec![];
        for mut sp in self.springs.drain(..) {
            if test(&sp) {
                if let Some(p) = &mut self.phys {
                    p.remove_spring(&mut sp);
                }
            } else {
                keep.push(sp);
            }
        }
        self.springs = keep;
    }

    fn remove_block(&mut self, i: usize) {
        let uid = self.blocks[i].uid;
        self.remove_cables_of(uid);
        self.remove_springs_where(|sp| sp.attached_to(uid));
        let mut b = self.blocks.remove(i);
        if let Some(p) = &mut self.phys {
            p.remove_block(&mut b);
        }
        self.refresh_mask();
    }

    fn crumble(&mut self, i: usize, into: Mat, thin: bool) {
        let b = self.blocks[i].clone();
        let (x0, y0, x1, y1) = b.bounds();
        self.remove_block(i);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if b.covers(vec2(x as f32 + 0.5, y as f32 + 0.5)) && (!thin || (x + y) % 2 == 0) && !self.world.raw(x, y).is_hard() {
                    self.world.set(x, y, into);
                }
            }
        }
    }

    /// A blow to a glass block: whole → two halves → four chips → shards.
    fn damage_block(&mut self, i: usize) {
        if i >= self.blocks.len() || self.blocks[i].hurt > 0 {
            return;
        }
        let b = self.blocks[i].clone();
        let (next, offsets): (Option<BlockKind>, Vec<Vec2>) = match b.kind {
            BlockKind::Glass => (Some(BlockKind::GlassHalf), vec![vec2(-5.0, 0.0), vec2(5.0, 0.0)]),
            BlockKind::GlassHalf => (Some(BlockKind::GlassChip), vec![vec2(-2.5, -2.5), vec2(2.5, -2.5), vec2(-2.5, 2.5), vec2(2.5, 2.5)]),
            _ => (None, vec![]),
        };
        let Some(kind) = next else {
            self.break_block(i);
            return;
        };
        self.checkpoint_quiet();
        self.remove_block(i);
        for off in offsets {
            let mut piece = Block::new(kind, b.pos + off_rot(b.angle, off), b.angle);
            piece.hurt = 12;
            if let Some(p) = &mut self.phys {
                p.add_block(&mut piece);
                // The pieces fly apart a little.
                p.knock(&piece, piece.pos, off_rot(b.angle, off).normalize_or_zero() * 0.6 + vec2(0.0, -0.3));
            }
            self.blocks.push(piece);
        }
        self.refresh_mask();
        self.sounds.emit(Ev::Clink);
        self.booms.push(Boom { pos: b.pos, t: 0.6, color: Color::new(0.7, 0.9, 1.0, 1.0) });
    }

    fn break_block(&mut self, i: usize) {
        let pos = self.blocks[i].pos;
        self.sounds.emit(Ev::Clink);
        self.crumble(i, Mat::Shard, true);
        self.booms.push(Boom { pos, t: 0.55, color: Color::new(0.7, 0.9, 1.0, 1.0) });
    }

    fn hurt_cooldown_clear(&mut self, i: usize) {
        self.blocks[i].hurt = 0;
    }

    fn burn_block(&mut self, i: usize) {
        self.crumble(i, Mat::Ember, false);
    }

    fn cable_click(&mut self, mat: CableMat, at: Vec2) {
        // A pulley on the way: the rope is threaded over it.
        if let Some(pi) = self.pulley_at(at) {
            if self.cable_from.is_none() {
                self.say("Fixe d'abord un bout du câble, puis clique les poulies par où il passe");
            } else {
                let uid = self.mech.pulleys[pi].uid;
                if self.cable_via.last() != Some(&uid) {
                    self.cable_via.push(uid);
                    self.say("Le câble passe sur la poulie : clique la suivante ou l'autre bout");
                }
            }
            return;
        }
        // A winch drum is a place to fix a rope: the winch then reels it in and out.
        let reel = self.reel_at(at).map(|gi| Anchor::Reel { uid: self.mech.gears[gi].uid, pos: self.mech.gears[gi].pos });
        let hit = reel.or_else(|| node_at(&self.world, &self.blocks, &self.members, at));
        // The first end must be fixed; the second may be left loose, hanging in the air.
        let loose = hit.is_none() && self.cable_from.is_some();
        let Some(anchor) = hit.or(loose.then_some(Anchor::Free(at))) else {
            self.say("Accroche le câble sur un bloc, une tige, le sol, un treuil ou le bord du monde");
            return;
        };
        if loose {
            self.say("Câble à bout libre : il pend de son autre extrémité");
        }
        // On a wheel the rope is fixed at the rim, so that turning the wheel winds it.
        let anchor = match anchor {
            Anchor::Block { uid, local } => match self.blocks.iter().find(|b| b.uid == uid) {
                Some(b) if b.kind == BlockKind::Wheel => {
                    let dir = if local.length() > 0.5 { local.normalize() } else { vec2(-1.0, 0.0) };
                    Anchor::Block { uid, local: dir * (b.spec().w / 2.0) }
                }
                _ => anchor,
            },
            other => other,
        };
        match self.cable_from.take() {
            None => self.cable_from = Some(anchor),
            Some(first) => {
                let (Some(a), Some(b)) = (anchor_world(&first, &self.blocks, &self.members), anchor_world(&anchor, &self.blocks, &self.members)) else { return };
                // The length is that of the whole path, over any pulleys.
                let via_pts = self.via_contacts(a, &self.cable_via.clone(), b);
                let path: Vec<Vec2> = std::iter::once(a).chain(via_pts.iter().copied()).chain(std::iter::once(b)).collect();
                let chord: f32 = path.windows(2).map(|w| (w[1] - w[0]).length()).sum();
                if chord < 10.0 {
                    self.say("Tire le câble plus loin avant de l'accrocher");
                    self.cable_from = Some(first);
                    return;
                }
                let units = mat.cost(chord * 1.06);
                let free = self.free_now();
                if !free {
                    self.census = self.world.census();
                    let have: u32 = mat.cost_from().iter().map(|m| self.census[*m as usize]).sum();
                    if have < units {
                        self.say(format!("Pas assez de {} pour ce câble : il en faut {units}, il y en a {have}", info(mat.cost_from()[0]).name.to_lowercase()));
                        self.cable_from = Some(first);
                        return;
                    }
                }
                self.checkpoint();
                if !free {
                    self.world.consume(mat.cost_from(), units, (a.x as i32, a.y as i32));
                }
                let mut c = Cable::new(first, anchor, chord).made_of(mat, self.cable_soft);
                c.via = std::mem::take(&mut self.cable_via);
                c.via_pts = via_pts;
                if let Some(p) = &mut self.phys {
                    p.add_cable(&self.blocks, &self.members, &mut c);
                }
                self.cables.push(c);
            }
        }
    }

    /// Three clicks: fix the first end, set the rest length, then pull the far end
    /// out to where it should be attached (or leave it loose by clicking in the air).
    fn spring_click(&mut self, kind: SpringKind, at: Vec2) {
        let Some(mut d) = self.spring_draft.take() else {
            match node_at(&self.world, &self.blocks, &self.members, at) {
                Some(a) => {
                    if let Some(p) = anchor_world(&a, &self.blocks, &self.members) {
                        self.spring_draft = Some(SpringDraft { a, pa: p, rest: None });
                    }
                }
                None => self.say("Fixe le premier bout sur un bloc, une tige, le sol ou le bord du monde"),
            }
            return;
        };
        match d.rest {
            None => {
                let len = (at - d.pa).length().min(SPRING_MAX_LEN);
                if len < 10.0 {
                    self.say("Éloigne le deuxième clic : il donne la longueur au repos");
                    self.spring_draft = Some(d);
                    return;
                }
                let end = d.pa + (at - d.pa).clamp_length_max(SPRING_MAX_LEN);
                d.rest = Some((end, len));
                self.spring_draft = Some(d);
            }
            Some((rest_pt, rest)) => {
                // Third click: attach the far end where it was pulled to, or leave it at rest.
                let (b, p1) = match node_at(&self.world, &self.blocks, &self.members, at) {
                    Some(a) => match anchor_world(&a, &self.blocks, &self.members) {
                        Some(p) if (p - d.pa).length() <= SPRING_MAX_LEN * 2.5 && a != d.a => (a, p),
                        _ => (Anchor::Free(rest_pt), rest_pt),
                    },
                    None => (Anchor::Free(rest_pt), rest_pt),
                };
                let units = kind.cost(rest);
                let free = self.free_now();
                if !free {
                    self.census = self.world.census();
                    let have: u32 = kind.cost_from().iter().map(|m| self.census[*m as usize]).sum();
                    if have < units {
                        self.say(format!("Pas assez de {} : il en faut {units}, il y en a {have}", info(kind.cost_from()[0]).name.to_lowercase()));
                        self.spring_draft = Some(d);
                        return;
                    }
                }
                self.checkpoint();
                if !free {
                    self.world.consume(kind.cost_from(), units, (d.pa.x as i32, d.pa.y as i32));
                }
                let mut sp = Spring::new(kind, d.a, b, rest, self.cable_soft, d.pa, p1);
                if let Some(p) = &mut self.phys {
                    p.add_spring(&self.blocks, &self.members, &mut sp);
                }
                self.springs.push(sp);
            }
        }
    }

    fn cable_points(&self, c: &Cable) -> Vec<Vec2> {
        if !c.pts.is_empty() {
            return c.pts.clone();
        }
        match (anchor_world(&c.a, &self.blocks, &self.members), anchor_world(&c.b, &self.blocks, &self.members)) {
            (Some(a), Some(b)) if c.via.is_empty() => cable_preview(a, b, c.len),
            (Some(a), Some(b)) => std::iter::once(a).chain(self.via_contacts(a, &c.via, b)).chain(std::iter::once(b)).collect(),
            _ => vec![],
        }
    }

    // ----- free beams -------------------------------------------------------

    /// Beams pinned to a beam that no longer exists become loose ends.
    fn fix_dangling_anchors(&mut self) {
        let uids: Vec<u32> = self.members.iter().map(|m| m.uid).collect();
        let positions: Vec<(u32, Vec2, Vec2)> = self.members.iter().map(|m| (m.uid, m.p0, m.p1)).collect();
        let _ = positions;
        for i in 0..self.members.len() {
            for end in 0..2 {
                let anchor = if end == 0 { self.members[i].a } else { self.members[i].b };
                if let Anchor::End { uid, .. } = anchor {
                    if !uids.contains(&uid) {
                        let p = if end == 0 { self.members[i].p0 } else { self.members[i].p1 };
                        if end == 0 {
                            self.members[i].a = Anchor::Free(p);
                        } else {
                            self.members[i].b = Anchor::Free(p);
                        }
                    }
                }
            }
        }
    }

    /// Cables fixed to a rod that is gone are cut loose.
    fn cut_cables_of_member(&mut self, uid: u32) {
        self.remove_springs_where(|sp| sp.attached_to_member(uid));
        let mut keep = vec![];
        for mut c in self.cables.drain(..) {
            if c.attached_to_member(uid) {
                if let Some(p) = &mut self.phys {
                    p.remove_cable(&mut c);
                }
            } else {
                keep.push(c);
            }
        }
        self.cables = keep;
    }

    fn remove_member_at(&mut self, i: usize) {
        let mut m = self.members.remove(i);
        self.cut_cables_of_member(m.uid);
        if let Some(p) = &mut self.phys {
            p.remove_member(&mut m);
        }
        self.fix_dangling_anchors();
        self.refresh_mask();
    }

    /// Where the far end of a beam started at `p0` would land for a cursor at `at`:
    /// on a nearby node if there is one, otherwise free, never longer than the maximum.
    /// The exact point a click at `at` would attach to (block, rod, ground or world edge).
    fn snap_point(&self, at: Vec2) -> Option<Vec2> {
        let a = node_at(&self.world, &self.blocks, &self.members, at)?;
        anchor_world(&a, &self.blocks, &self.members)
    }

    /// Shows where an end will be fixed: a dot on the anchor, a thread to it from the cursor,
    /// and the world edge lit up when the cursor is close enough to stick to it.
    fn draw_anchor_hint(&self, ui: &Ui, view: View, at: Vec2) {
        let (w, h) = (W as f32, H as f32);
        let near = blocks::BORDER_REACH;
        let lit = Color::new(1.0, 0.85, 0.3, 0.8);
        let edge = |a: Vec2, b: Vec2| {
            let (sa, sb) = (view.to_screen(a), view.to_screen(b));
            draw_line(sa.x, sa.y, sb.x, sb.y, 3.0, lit);
        };
        if at.x < near {
            edge(vec2(0.0, 0.0), vec2(0.0, h));
        }
        if at.x > w - near {
            edge(vec2(w, 0.0), vec2(w, h));
        }
        if at.y < near {
            edge(vec2(0.0, 0.0), vec2(w, 0.0));
        }
        if at.y > h - near {
            edge(vec2(0.0, h), vec2(w, h));
        }
        let Some(p) = self.snap_point(at) else { return };
        let (c, s) = (ui.mouse, view.to_screen(p));
        if (c - s).length() > 3.0 {
            draw_line(c.x, c.y, s.x, s.y, 1.5, Color::new(1.0, 1.0, 1.0, 0.7));
        }
        draw_circle(s.x, s.y, 5.5, Color::new(0.0, 0.0, 0.0, 0.55));
        draw_circle(s.x, s.y, 4.0, GREEN);
        draw_circle_lines(s.x, s.y, 8.0, 2.0, WHITE);
    }

    fn beam_end(&self, p0: Vec2, at: Vec2, start: &Anchor) -> (Anchor, Vec2) {
        if let Some(a) = node_at(&self.world, &self.blocks, &self.members, at) {
            if &a != start {
                if let Some(p) = anchor_world(&a, &self.blocks, &self.members) {
                    if (p - p0).length() <= MEMBER_MAX_LEN {
                        return (a, p);
                    }
                }
            }
        }
        let d = at - p0;
        let p = p0 + d.clamp_length_max(MEMBER_MAX_LEN);
        (Anchor::Free(p), p)
    }

    fn beam_click(&mut self, mat: MemberMat, at: Vec2) {
        match self.member_from.take() {
            None => match node_at(&self.world, &self.blocks, &self.members, at) {
                Some(a) => {
                    if let Some(p) = anchor_world(&a, &self.blocks, &self.members) {
                        self.member_from = Some((a, p));
                    }
                }
                None => {
                    // No attachment: the rod is allowed, and it will simply fall.
                    self.member_from = Some((Anchor::Free(at), at));
                    self.say("Sans attache : la tige tombera");
                }
            },
            Some((a, p0)) => {
                let (b, p1) = self.beam_end(p0, at, &a);
                let len = (p1 - p0).length();
                if len < 12.0 {
                    self.say("Tire la poutre plus loin");
                    self.member_from = Some((a, p0));
                    return;
                }
                let units = mat.cost(len);
                let free = self.free_now();
                if !free {
                    self.census = self.world.census();
                    let have: u32 = mat.cost_from().iter().map(|m| self.census[*m as usize]).sum();
                    if have < units {
                        self.say(format!("Pas assez de {} : il en faut {units}, il y en a {have}. Peins-en !", mat.name().to_lowercase()));
                        self.member_from = Some((a, p0));
                        return;
                    }
                }
                self.checkpoint();
                if !free {
                    self.world.consume(mat.cost_from(), units, (p0.x as i32, p0.y as i32));
                }
                self.members.push(Member::new(mat, a, b, p0, p1));
                let idx = self.members.len() - 1;
                if let Some(p) = &mut self.phys {
                    p.add_member(&self.blocks, &mut self.members, idx);
                }
                self.refresh_mask();
                // Carry on from the new node, like drawing a truss.
                self.member_from = Some((Anchor::End { uid: self.members[idx].uid, end: 1 }, p1));
            }
        }
    }

    // ----- hammer -----------------------------------------------------------

    fn hammer(&mut self, at: Vec2) {
        self.hammer_fx.push((at, 0.0));
        if let Some(i) = block_at(&self.blocks, at) {
            if self.blocks[i].kind.is_glass() {
                self.hurt_cooldown_clear(i);
                self.damage_block(i);
            } else {
                if self.phys.is_none() {
                    self.build_physics();
                }
                let b = self.blocks[i].clone();
                let dir = (b.pos - at).normalize_or_zero();
                if let Some(p) = &mut self.phys {
                    p.knock(&b, at, vec2(dir.x, dir.y - 0.4));
                }
                self.say("Le bloc est secoué (lance la simulation pour le voir bouger)");
            }
            return;
        }
        let (cx, cy) = (at.x as i32, at.y as i32);
        let mut hit = false;
        for dy in -3..=3 {
            for dx in -3..=3 {
                match self.world.raw(cx + dx, cy + dy) {
                    Mat::Glass => {
                        self.world.shatter(cx + dx, cy + dy);
                        hit = true;
                    }
                    Mat::Ice => {
                        self.world.set(cx + dx, cy + dy, Mat::Snow);
                        hit = true;
                    }
                    _ => {}
                }
            }
        }
        if !hit {
            self.say("Le marteau casse le verre et la glace");
        }
    }

    // ----- tuning -----------------------------------------------------------

    /// The tuning being edited: the selected car on track, else the next cars.
    fn tuning_mut(&mut self) -> &mut Tuning {
        let live = self.selected_car.and_then(|id| self.cars.iter().position(|c| c.id == id));
        match live {
            Some(i) => &mut self.cars[i].tuning,
            None => {
                let i = self.model_index();
                &mut self.tunings[i]
            }
        }
    }

    fn sync_default_tuning(&mut self) {
        if let Some(id) = self.selected_car {
            if let Some(c) = self.cars.iter().find(|c| c.id == id) {
                let t = c.tuning;
                let i = ModelKind::ALL.iter().position(|k| *k == c.kind).unwrap_or(0);
                self.tunings[i] = t;
            }
        }
    }

    // ----- input ------------------------------------------------------------

    fn handle_keys(&mut self) {
        if self.quit_modal {
            if is_key_pressed(KeyCode::Left) {
                self.quit_choice = false;
            } else if is_key_pressed(KeyCode::Right) {
                self.quit_choice = true;
            } else if is_key_pressed(KeyCode::Tab) {
                self.quit_choice = !self.quit_choice;
            }
            if is_key_pressed(KeyCode::Escape) {
                self.on_escape();
            } else if (is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter)) && self.on_enter_in_quit_modal() {
                Self::quit_game();
            }
            return;
        }
        if is_key_pressed(KeyCode::Escape) {
            self.on_escape();
        }
        if (is_key_pressed(KeyCode::Delete) || is_key_pressed(KeyCode::Backspace)) && self.tool == Tool::Select {
            self.select_delete();
        }
        if is_key_pressed(KeyCode::M) {
            self.sounds.toggle_mute();
            self.say(if self.sounds.muted { "Son coupé (M)" } else { "Son activé (M)" });
        }
        if is_key_pressed(KeyCode::Space) {
            let run = !self.running;
            self.set_running(run);
        }
        if is_key_pressed(KeyCode::R) {
            self.reset();
        }
        if is_key_pressed(KeyCode::Z) {
            self.undo();
        }
        if is_key_pressed(KeyCode::G) {
            let b = Biome::ALL[(self.world.biome.id() as usize + 1) % 3];
            self.new_world(b);
        }
        if is_key_pressed(KeyCode::N) {
            self.empty_world();
        }
        if is_key_pressed(KeyCode::C) {
            self.launch_car();
        }
        if is_key_pressed(KeyCode::H) {
            self.mode = if self.mode == ViewMode::Thermal { ViewMode::Normal } else { ViewMode::Thermal };
        }
        if is_key_pressed(KeyCode::X) {
            self.mode = if self.mode == ViewMode::Stress { ViewMode::Normal } else { ViewMode::Stress };
        }
        if is_key_pressed(KeyCode::V) {
            self.mode = if self.mode == ViewMode::Wind { ViewMode::Normal } else { ViewMode::Wind };
        }
        if is_key_pressed(KeyCode::B) {
            if self.planes.plane.is_none() {
                self.planes.summon(&self.world);
                self.say("Un Béluga passe dans le ciel !");
            }
        }
        if is_key_pressed(KeyCode::W) {
            if self.whales.summon(&self.world) {
                self.say("Une baleine remonte respirer !");
            } else if self.whales.whale.is_none() {
                self.say("Il faut une grande étendue d'eau profonde pour une baleine");
            }
        }
        if is_key_pressed(KeyCode::Y) {
            self.mode = if self.mode == ViewMode::Humidity { ViewMode::Normal } else { ViewMode::Humidity };
        }
        if is_key_pressed(KeyCode::P) {
            self.mode = if self.mode == ViewMode::Pressure { ViewMode::Normal } else { ViewMode::Pressure };
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
                Tab::Planet => Tab::Matter,
                Tab::Matter => Tab::Blocks,
                Tab::Blocks => Tab::Mech,
                Tab::Mech => Tab::Fluid,
                Tab::Fluid => Tab::Cars,
                Tab::Cars => Tab::Planet,
            };
        }
        let digits = [
            KeyCode::Key1, KeyCode::Key2, KeyCode::Key3, KeyCode::Key4, KeyCode::Key5,
            KeyCode::Key6, KeyCode::Key7, KeyCode::Key8, KeyCode::Key9, KeyCode::Key0,
        ];
        for (i, k) in digits.iter().enumerate() {
            if is_key_pressed(*k) {
                self.tool = Tool::Mat(MATTERS[i].states[self.matter_state[i]]);
                self.tab = Tab::Matter;
            }
        }
    }

    /// Ctrl + wheel zooms on the pointer, Ctrl + drag moves the view (`base` is the standard view).
    fn handle_camera(&mut self, base: View, ui: &Ui) {
        let left = is_mouse_button_down(MouseButton::Left);
        if self.show_science || self.quit_modal {
            self.camera.drag(base, ui.mouse, false, false);
            return;
        }
        let over_ui = self.panel_hit.contains(ui.mouse) || self.views_hit.contains(ui.mouse) || self.tools_hit.contains(ui.mouse) || self.corner_hit.contains(ui.mouse) || self.hud_hit.contains(ui.mouse) || self.launch_hit.contains(ui.mouse);
        let inside = !over_ui && base.visible().contains(ui.mouse);
        let pressed = is_mouse_button_pressed(MouseButton::Left);
        if self.camera.dragging() || (ctrl_down() && inside && pressed) {
            self.camera.drag(base, ui.mouse, pressed, left);
        }
        let wheel = mouse_wheel().1;
        if ctrl_down() && inside && wheel != 0.0 {
            self.camera.zoom_at(base, ui.mouse, wheel.signum());
            let z = self.camera.zoom();
            self.say(if z > 1.0 { format!("Zoom ×{z:.1} (Échap : vue standard)") } else { "Vue standard".to_string() });
        }
    }

    fn handle_mouse(&mut self, view: View, ui: &Ui) {
        if self.show_science || self.quit_modal {
            return;
        }
        // Ctrl belongs to the camera: no painting or building while zooming or moving the view.
        if ctrl_down() || self.camera.dragging() {
            self.last = None;
            return;
        }
        let over_ui = self.panel_hit.contains(ui.mouse) || self.views_hit.contains(ui.mouse) || self.tools_hit.contains(ui.mouse) || self.corner_hit.contains(ui.mouse) || self.hud_hit.contains(ui.mouse) || self.launch_hit.contains(ui.mouse);
        let inside = !over_ui && view.visible().contains(ui.mouse);
        let wheel = mouse_wheel().1;
        if inside && wheel != 0.0 {
            let d = if wheel > 0.0 { 1 } else { -1 };
            self.brush = (self.brush + d).clamp(0, 40);
        }
        let left = is_mouse_button_down(MouseButton::Left);
        let right = is_mouse_button_down(MouseButton::Right);
        if !inside || (!left && !right) {
            if !left && !right {
                // The shovel lets go where the pointer was when the button went up.
                if let Some(at) = self.last {
                    self.shovel_release(at);
                }
                self.shovel_charge = None;
                self.last = None;
                self.drag = None;
            }
            return;
        }
        let at = view.to_world(ui.mouse);
        let g = (at.x.floor() as i32, at.y.floor() as i32);
        let from = self.last.map(|p| (p.x.floor() as i32, p.y.floor() as i32)).unwrap_or(g);
        self.last = Some(at);
        let click = is_mouse_button_pressed(MouseButton::Left);
        let rclick = is_mouse_button_pressed(MouseButton::Right);

        if rclick && (self.cable_from.is_some() || self.member_from.is_some() || self.spring_draft.is_some() || self.mech_draft.is_some() || self.pipe_from.is_some()) {
            self.mech_draft = None;
            self.pipe_from = None;
            self.cable_from = None;
            self.cable_via.clear();
            self.member_from = None;
            self.spring_draft = None;
            return;
        }
        if self.tool == Tool::Shovel && right {
            if rclick {
                self.checkpoint();
                self.shovel_dump(at);
            }
            return;
        }
        if right || (left && self.tool == Tool::Eraser) {
            if click || rclick {
                if let Some(i) = block_at(&self.blocks, at) {
                    self.remove_block(i);
                    return;
                }
                if self.mech_erase(at) || self.fluid_erase(at) {
                    return;
                }
                if let Some(i) = self.members.iter().position(|m| m.distance_to(at) < m.mat.thickness() / 2.0 + 3.0) {
                    self.remove_member_at(i);
                    return;
                }
                if let Some(i) = self.springs.iter().position(|sp| sp.distance_to(at) < 5.0) {
                    let mut sp = self.springs.remove(i);
                    if let Some(p) = &mut self.phys {
                        p.remove_spring(&mut sp);
                    }
                    return;
                }
                if let Some(i) = self.cables.iter().position(|c| c.distance_to(at) < 4.0) {
                    let mut c = self.cables.remove(i);
                    if let Some(p) = &mut self.phys {
                        p.remove_cable(&mut c);
                    }
                    return;
                }
            }
            self.world.line(from, g, self.brush.max(2), Mat::Empty);
            return;
        }
        match self.tool {
            Tool::Mat(m) => {
                let rule = match self.paint_mode {
                    PaintMode::Add => PaintRule::Fill,
                    PaintMode::Replace => PaintRule::Replace,
                    PaintMode::Only => {
                        // The material under the first click is the one being replaced.
                        if click {
                            self.only_target = self.world.raw(g.0, g.1);
                        }
                        PaintRule::Only(self.only_target)
                    }
                };
                self.world.line_rule(from, g, self.brush, m, rule);
            }
            Tool::Block(k) if click => self.place_block(k, at),
            Tool::Cable(m) if click => self.cable_click(m, at),
            Tool::Spring(k) if click => self.spring_click(k, at),
            Tool::Beam(m) if click => self.beam_click(m, at),
            Tool::Gear(_) | Tool::Motor | Tool::Axle | Tool::Rail | Tool::Crank | Tool::Pivot | Tool::Screw | Tool::Winch | Tool::Pulley if click => self.mech_click(at),
            Tool::Pipe | Tool::Valve | Tool::Pump if click => self.fluid_click(at),
            Tool::Shovel => self.shovel_mouse(at, click, left),
            Tool::Select => self.select_mouse(at, click, left, is_key_down(KeyCode::LeftShift)),
            Tool::Hammer if click => self.hammer(at),
            Tool::Sensor if click => self.sensor_click(at),
            Tool::Entry if click => self.entry = Some(g),
            Tool::Exit if click => self.exit = Some(g),
            _ => {}
        }
    }

    // ----- simulation -------------------------------------------------------

    fn update(&mut self, dt: f32) {
        self.time += dt;
        self.shake *= 0.88f32.powf(dt * 60.0);
        if self.shake < 0.15 {
            self.shake = 0.0;
        }
        // Keep the game smooth: if frames stay slow while simulating, run fewer steps per frame.
        self.frame_ema = self.frame_ema * 0.92 + dt * 0.08;
        if self.running && self.frame_ema > 0.026 {
            self.slow_frames += 1;
        } else {
            self.slow_frames = self.slow_frames.saturating_sub(2);
        }
        // First simplify the physics (cheaper approximations that keep the look of the thing), and
        // only when that is not enough run fewer steps per frame.
        self.lod_lock = (self.lod_lock - dt).max(0.0);
        if self.slow_frames > 60 {
            self.slow_frames = 0;
            self.frame_ema = 0.016;
            self.fast_frames = 0;
            if self.world.lod < 3 {
                self.world.lod += 1;
                // A simplification that has to come back straight away: do not try to undo it for a while.
                self.lod_lock = 300.0;
                self.say(match self.world.lod {
                    1 => "Simplification 1/3 : la chaleur de l'air est calculée par blocs de 2×2",
                    2 => "Simplification 2/3 : la chaleur est calculée une fois sur deux (vitesses compensées)",
                    _ => "Simplification 3/3 : l'air calme ne conduit plus la chaleur",
                }
                .to_string());
            } else if self.speed > 1 {
                self.speed -= 1;
                self.say(format!("Vitesse réduite à x{} pour rester fluide", self.speed));
            }
        }
        // Long smooth stretches bring the exact simulation back, one level at a time.
        if self.running && self.world.lod > 0 && self.frame_ema < 0.011 && self.lod_lock <= 0.0 {
            self.fast_frames += 1;
            if self.fast_frames > 900 {
                self.fast_frames = 0;
                self.world.lod -= 1;
                self.frame_ema = 0.016;
                self.say(if self.world.lod == 0 { "Simulation exacte rétablie".to_string() } else { format!("Simplification allégée : niveau {}/3", self.world.lod) });
            }
        } else {
            self.fast_frames = 0;
        }
        let building = matches!(self.tool, Tool::Block(_) | Tool::Cable(_) | Tool::Spring(_) | Tool::Beam(_) | Tool::Gear(_) | Tool::Motor | Tool::Axle | Tool::Rail | Tool::Crank | Tool::Pivot | Tool::Screw | Tool::Winch | Tool::Pulley | Tool::Pipe | Tool::Valve | Tool::Pump);
        if (self.tab == Tab::Blocks || building) && (self.time * 4.0) as u32 != ((self.time - dt) * 4.0) as u32 {
            self.census = self.world.census();
        }
        self.shovel_age += dt;
        if let Some(at) = self.planes.update(dt, &self.world) {
            self.world.explode(at.x as i32, at.y as i32, 14);
            self.say("Le Béluga s'est écrasé !".to_string());
        }
        if self.whales.update(dt, &self.world) {
            self.sounds.emit(Ev::Blow);
        }
        self.fireworks.update(dt);
        if self.fireworks.take_bursts() > 0 {
            self.sounds.emit(Ev::Pop);
        }
        for b in &mut self.booms {
            b.t += dt / 0.7;
        }
        self.booms.retain(|b| b.t < 1.0);
        for h in &mut self.hammer_fx {
            h.1 += dt / 0.35;
        }
        self.hammer_fx.retain(|h| h.1 < 1.0);
        if let Some((_, t)) = &mut self.toast {
            *t -= dt;
        }
        if self.toast.as_ref().map_or(false, |t| t.1 <= 0.0) {
            self.toast = None;
        }
        if !self.running {
            let (blocks, members) = (&self.blocks, &self.members);
            self.mech.follow_mounts(|a| anchor_world(a, blocks, members));
            self.refresh_reel_anchors();
            self.sounds.update(dt, &Mix::default());
            return;
        }
        // A fixed time step: 60 steps a second at x1, the same on a 60 Hz or a 144 Hz screen. A
        // machine that cannot keep up catches up a little, then lets the time go (and the frame
        // watch above simplifies the physics or lowers the speed).
        self.sim_acc += dt * STEPS_PER_SECOND * self.speed as f32;
        let owed = self.sim_acc.floor() as usize;
        let max = self.speed * 2;
        let steps = owed.min(max);
        self.sim_acc = if owed > max { 0.0 } else { self.sim_acc - steps as f32 };
        for _ in 0..steps {
            self.sim_step();
        }
        if (self.time * 2.0) as u32 != ((self.time - dt) * 2.0) as u32 {
            let n = self.world.census();
            self.heat_cells = n[Mat::Fire as usize] + n[Mat::Lava as usize] / 3 + n[Mat::Ember as usize] / 2;
        }
        let rpm = self
            .cars
            .iter()
            .filter(|c| c.fate == Fate::Driving)
            .map(|c| 0.12 + 0.88 * (c.v.length() / c.kind.model().top_speed).min(1.0))
            .fold(None, |a: Option<f32>, r| Some(a.map_or(r, |a| a.max(r))));
        let mix = Mix {
            wind: ((self.world.wind.abs() - 0.55) / 0.45).clamp(0.0, 1.0),
            water: (self.world.take_water_flow() as f32 / (dt * 3600.0 * self.speed as f32).max(1.0)).sqrt().min(1.0),
            fire: (self.heat_cells as f32 / 150.0).sqrt().min(1.0),
            rpm,
        };
        self.sounds.update(dt, &mix);
    }

    fn sim_step(&mut self) {
        self.world.step();
        self.sensors.step(&self.world);
        let exit = self.exit;
        let was_driving: Vec<bool> = self.cars.iter().map(|c| c.fate == Fate::Driving).collect();
        for c in &mut self.cars {
            c.step(&self.world, exit);
        }
        let (clang, smoke) = self.debris.step(&self.world);
        if clang > 0.5 {
            self.sounds.emit(Ev::Thud(clang * 0.5));
        }
        for (x, y) in smoke {
            if self.world.raw(x, y) == Mat::Empty {
                self.world.set(x, y, Mat::Smoke);
            }
        }
        // Cars that meet either bump gently or wreck each other on the spot.
        for i in 0..self.cars.len() {
            for j in i + 1..self.cars.len() {
                let (left, right) = self.cars.split_at_mut(j);
                collide_cars(&mut left[i], &mut right[0]);
            }
        }

        let mut flung = false;
        for c in &self.cars {
            for &(pos, vel) in &c.sprays {
                let to = pos + vel * 3.5;
                flung |= self.world.fling((pos.x as i32, pos.y as i32), (to.x as i32, to.y as i32));
            }
        }
        if flung {
            self.sounds.emit(Ev::Spray);
        }
        for (c, was) in self.cars.iter().zip(was_driving) {
            if was && c.fate == Fate::Exploded {
                self.sounds.emit(Ev::Boom(8.0));
            }
        }

        // Cars smash glass (cells and bricks) when they hit it fast enough.
        let smashed: Vec<(i32, i32)> = self.cars.iter().flat_map(|c| c.smashed.iter().copied()).collect();
        let mut glass_blocks: Vec<usize> = vec![];
        for (x, y) in smashed {
            match self.world.block_at_cell(x, y) {
                Some(id) if self.blocks.get(id).map_or(false, |b| b.kind.is_glass()) => {
                    if !glass_blocks.contains(&id) {
                        glass_blocks.push(id);
                    }
                }
                Some(_) => {}
                None => self.world.shatter(x, y),
            }
        }
        let uids: Vec<u32> = glass_blocks.iter().filter_map(|&id| self.blocks.get(id).map(|b| b.uid)).collect();
        for uid in uids {
            if let Some(i) = self.blocks.iter().position(|b| b.uid == uid) {
                self.damage_block(i);
            }
        }

        // Cars weigh on the blocks they drive over (bridges can give way).
        let mut impacts: Vec<u32> = vec![];
        if let Some(p) = &mut self.phys {
            for c in &self.cars {
                let weight = c.weight_kg();
                for (pt, share) in &c.loads {
                    if let Some(id) = self.world.block_at_cell(pt.x.floor() as i32, pt.y.floor() as i32) {
                        if let Some(b) = self.blocks.get(id) {
                            p.press(b, *pt, weight * share);
                        } else if let Some(m) = self.members.get(id - self.blocks.len()) {
                            p.press_member(m, *pt, weight * share);
                        }
                    }
                }
            }
            p.step(&self.world, &mut self.blocks);
            impacts = p.take_impacts();
            for (_, hard) in p.take_bumps() {
                self.sounds.emit(Ev::Thud(hard));
            }
            let broken = p.update_members(&self.world, &mut self.members);
            p.sync_cables(&self.blocks, &self.members, &mut self.cables);
            for _gone in p.sync_springs(&self.blocks, &self.members, &mut self.springs) {
                self.sounds.emit(Ev::Snap);
                self.booms.push(Boom { pos: self.cars.first().map_or(vec2(0.0, 0.0), |c| c.p), t: 0.9, color: Color::new(1.0, 1.0, 1.0, 1.0) });
            }
            for (spot, uid) in broken {
                self.sounds.emit(Ev::Crack);
                self.booms.push(Boom { pos: spot, t: 0.5, color: Color::new(0.9, 0.7, 0.4, 1.0) });
                self.beam_broke = true;
                self.broken_members.push(uid);
            }
        }
        for uid in impacts {
            if let Some(i) = self.blocks.iter().position(|b| b.uid == uid) {
                self.damage_block(i);
            }
        }
        for b in &mut self.blocks {
            b.hurt = b.hurt.saturating_sub(1);
        }
        if self.beam_broke {
            self.beam_broke = false;
            for uid in std::mem::take(&mut self.broken_members) {
                self.cut_cables_of_member(uid);
            }
            self.fix_dangling_anchors();
        }
        self.mech_step();
        self.fluid_step();
        stamp_all(&mut self.world, &self.blocks, &self.members);

        // Flames heat wooden blocks; water rusts metal ones.
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
        let mut rusted: Vec<usize> = vec![];
        for (i, b) in self.blocks.iter_mut().enumerate() {
            let mat = b.spec().mat;
            if matches!(mat, Mat::Iron | Mat::Steel) {
                let reach = b.half().length() + 2.0;
                let wet = [vec2(reach, 0.0), vec2(-reach, 0.0), vec2(0.0, reach), vec2(0.0, -reach)]
                    .iter()
                    .filter(|d| self.world.raw((b.pos.x + d.x) as i32, (b.pos.y + d.y) as i32) == Mat::Water)
                    .count();
                if wet >= 2 {
                    b.rust += if mat == Mat::Iron { 0.0004 } else { 0.00012 };
                    if b.rust >= 1.0 {
                        rusted.push(i);
                    }
                }
            }
        }
        // Explosions shove blocks and wreck nearby cars.
        for (_, _, hard) in std::mem::take(&mut self.world.thuds) {
            self.sounds.emit(Ev::Thud(hard * 0.6));
        }
        let blasts = std::mem::take(&mut self.world.blasts);
        for (x, y, r) in blasts {
            let c = vec2(x as f32, y as f32);
            if let Some(p) = &mut self.phys {
                p.blast(&self.blocks, c.x, c.y, r as f32);
            }
            for (i, b) in self.blocks.iter_mut().enumerate() {
                let d = (b.pos - c).length();
                if b.spec().mat == Mat::Wood && d < r as f32 * 1.4 {
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
            self.sounds.emit(Ev::Boom(r as f32));
            self.booms.push(Boom { pos: c, t: 0.0, color: Color::new(1.0, 0.5, 0.1, 1.0) });
            // The pressure wave rattles the whole picture.
            self.shake = self.shake.max((r as f32 * 0.75).min(18.0));
        }
        // Gunpowder going off at night sends up a firework.
        let powder = std::mem::take(&mut self.world.powder_booms);
        if !powder.is_empty() && backdrop::daylight(self.world.phase()) < 0.3 {
            let at = powder.iter().fold(Vec2::ZERO, |a, &(x, y)| a + vec2(x as f32, y as f32)) / powder.len() as f32;
            if self.fireworks.launch(at, 2 + powder.len().min(4)) && self.fireworks_told != self.world.day_number() {
                self.fireworks_told = self.world.day_number();
                self.say("Feu d'artifice ! Ses couleurs viennent de métaux : cuivre bleu, strontium rouge, sodium jaune");
            }
        }
        match self.trucks.step(&mut self.world) {
            Some(firetruck::News::Siren) => {
                self.sounds.emit(Ev::Siren);
                if self.trucks.truck.as_ref().map_or(false, |t| t.age == 0) {
                    self.say("Pin-pon ! Les pompiers arrivent");
                }
            }
            Some(firetruck::News::Arrived) => self.say("Les pompiers arrosent le feu"),
            Some(firetruck::News::PutOut) => self.say("Les pompiers ont éteint le feu. Bravo !"),
            Some(firetruck::News::TankEmpty) => self.say("Le camion de pompiers a vidé sa citerne"),
            Some(firetruck::News::Blocked) => self.say("Le camion de pompiers ne peut pas passer !"),
            None => {}
        }
        // A wall gave way under the pressure of a closed pocket (a boiler): say how much it held.
        if let Some(&(_, _, atm)) = std::mem::take(&mut self.world.bursts).iter().max_by(|a, b| a.2.total_cmp(&b.2)) {
            self.say(format!("Une paroi a cédé sous la pression : {atm:.1} atm (l'air libre est à 1 atm)"));
        }
        let outside = |p: Vec2| p.y > H as f32 + 60.0 || p.x < -60.0 || p.x > W as f32 + 60.0;
        let mut gone: Vec<(usize, u8)> = vec![];
        for i in 0..self.blocks.len() {
            if outside(self.blocks[i].pos) {
                gone.push((i, 0));
            }
        }
        for i in burn {
            if !gone.iter().any(|g| g.0 == i) {
                gone.push((i, 1));
            }
        }
        for i in rusted {
            if !gone.iter().any(|g| g.0 == i) {
                gone.push((i, 2));
            }
        }
        gone.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        for (i, why) in gone {
            match why {
                0 => self.remove_block(i),
                1 => self.burn_block(i),
                _ => self.crumble(i, Mat::Rust, false),
            }
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
                    self.world.explode(c.p.x as i32, c.p.y as i32, 12);
                    self.debris.burst(&c);
                }
            }
        }
        self.cars = keep;
        if let Some(id) = self.selected_car {
            if !self.cars.iter().any(|c| c.id == id) {
                self.selected_car = None;
            }
        }
    }

    // ----- drawing ----------------------------------------------------------

    fn draw_scene(&mut self, view: View, renderer: &mut Renderer, ui: &Ui) {
        let phase = self.world.phase();
        let day = backdrop::daylight(phase);
        let thermal = self.mode == ViewMode::Thermal;
        // The thermal and pressure views paint the whole picture themselves.
        let flat = thermal || matches!(self.mode, ViewMode::Pressure | ViewMode::Humidity);
        let render_mode = match self.mode {
            ViewMode::Thermal => RenderMode::Thermal,
            ViewMode::Pressure => RenderMode::Pressure,
            ViewMode::Humidity => RenderMode::Humidity,
            _ => RenderMode::Normal,
        };
        if !flat {
            backdrop::draw_sky(view, self.world.biome, phase, self.world.wind, self.time, self.world.season());
        }
        renderer.draw_world(&self.world, view, render_mode, self.time);
        if !flat {
            renderer.draw_gas(&self.world, view);
            self.planes.draw(view, self.time);
            self.fireworks.draw(view, self.time);
            self.whales.draw(view);
            self.trucks.draw(view, self.time);
        }
        let stress = if self.mode == ViewMode::Stress { self.phys.as_ref().map(|p| p.stress(&self.blocks)) } else { None };
        for b in &self.blocks {
            let st = stress.as_ref().map(|m| m.get(&b.uid).copied().unwrap_or(0.0));
            draw_block(b, view, st);
        }
        for m in &self.members {
            draw_member(m, view, self.mode == ViewMode::Stress, self.time);
        }
        for sp in &self.springs {
            draw_spring(sp, view, self.mode == ViewMode::Stress);
        }
        for c in &self.cables {
            let pts = self.cable_points(c);
            let st = if self.mode == ViewMode::Stress { Some(c.stress) } else { None };
            draw_cable(&pts, view, c.snapped, true, c.mat.color(), st);
        }
        let over_ui_now = self.panel_hit.contains(ui.mouse) || self.views_hit.contains(ui.mouse) || self.tools_hit.contains(ui.mouse) || self.corner_hit.contains(ui.mouse) || self.hud_hit.contains(ui.mouse) || self.launch_hit.contains(ui.mouse);
        let inside_now = !over_ui_now && view.visible().contains(ui.mouse) && !self.show_science;
        self.draw_pipes(view, ui, inside_now);
        self.draw_mech(view, ui, inside_now);
        self.draw_selection(view);
        if let Some(e) = self.entry {
            draw_flag(view, e, GREEN, false);
        }
        if let Some(x) = self.exit {
            draw_flag(view, x, RED, true);
        }
        self.draw_sensors(ui, view);
        for c in &self.cars {
            draw_car(c, view);
            if self.selected_car == Some(c.id) && self.selection != Some(select::Sel::Car(c.id)) {
                select::outline_car(c, view, ACCENT);
            }
        }
        self.debris.draw(view);
        draw_booms(&self.booms, view);
        for (at, t) in &self.hammer_fx {
            let s = view.to_screen(*at);
            let angle = -1.1 + 1.6 * t;
            let head = s + vec2(angle.sin(), -angle.cos()) * 34.0;
            draw_line(s.x, s.y, head.x, head.y, 4.0, Color::from_rgba(150, 100, 60, 255));
            draw_circle(head.x, head.y, 9.0, Color::from_rgba(120, 124, 140, 255));
            draw_circle_lines(s.x, s.y, 6.0 + 26.0 * t, 2.0, Color::new(1.0, 1.0, 1.0, 1.0 - t));
        }
        if !flat {
            renderer.draw_night(view, day, self.time);
        } else if thermal {
            self.draw_sun_on_thermal(view, ui);
        }
        if self.mode == ViewMode::Wind {
            self.draw_wind_field(view);
        }

        // Cursor preview.
        let over_ui = self.panel_hit.contains(ui.mouse) || self.views_hit.contains(ui.mouse) || self.tools_hit.contains(ui.mouse) || self.corner_hit.contains(ui.mouse) || self.hud_hit.contains(ui.mouse) || self.launch_hit.contains(ui.mouse);
        let inside = !over_ui && view.visible().contains(ui.mouse) && !self.show_science;
        if let Some(a) = &self.cable_from {
            if let Some(p) = anchor_world(a, &self.blocks, &self.members) {
                let cursor = if inside { view.to_world(ui.mouse) } else { p };
                let snapped = if inside { self.snap_point(cursor) } else { None };
                let end = snapped.unwrap_or(cursor);
                let rgb = if let Tool::Cable(m) = self.tool { m.color() } else { (62, 66, 78) };
                if self.cable_via.is_empty() {
                    // A loose second end is fine, so the preview is never "refused".
                    let pts = cable_preview(p, end, (end - p).length());
                    draw_cable(&pts, view, false, true, rgb, None);
                } else {
                    // Straight between the pulleys the rope runs over.
                    let via = self.via_contacts(p, &self.cable_via, end);
                    let path: Vec<Vec2> = std::iter::once(p).chain(via).chain(std::iter::once(end)).collect();
                    draw_cable(&path, view, false, true, rgb, None);
                }
            }
        }
        if let (Tool::Spring(kind), Some(d)) = (self.tool, self.spring_draft.as_ref()) {
            let cursor = if inside { view.to_world(ui.mouse) } else { d.pa };
            match d.rest {
                None => {
                    let end = d.pa + (cursor - d.pa).clamp_length_max(SPRING_MAX_LEN);
                    draw_spring_between(d.pa, end, (end - d.pa).length(), kind, view, None, 0.7);
                    let label = format!("repos : {:.0}", (end - d.pa).length());
                    let s = view.to_screen(end);
                    ui.text(&label, s.x + 10.0, s.y - 8.0, 15, WHITE);
                }
                Some((rest_pt, rest)) => {
                    // The spring at rest as a ghost, and the stretched one that follows the cursor.
                    draw_spring_between(d.pa, rest_pt, rest, kind, view, None, 0.25);
                    let target = match node_at(&self.world, &self.blocks, &self.members, cursor) {
                        Some(a) => anchor_world(&a, &self.blocks, &self.members).unwrap_or(rest_pt),
                        None => rest_pt,
                    };
                    draw_spring_between(d.pa, target, rest, kind, view, None, 0.9);
                    let s = view.to_screen(target);
                    draw_circle_lines(s.x, s.y, 5.0 * view.s, 2.0, if target != rest_pt { GREEN } else { ACCENT });
                }
            }
        }
        if let (Tool::Beam(mat), Some((a, p0))) = (self.tool, self.member_from) {
            let at = if inside { view.to_world(ui.mouse) } else { p0 };
            let (_, p1) = self.beam_end(p0, at, &a);
            let len = (p1 - p0).length();
            let (cr, cg, cb) = mat.color();
            let ok = len >= 12.0;
            let col = if ok { Color::from_rgba(cr, cg, cb, 255) } else { Color::new(1.0, 0.3, 0.3, 1.0) };
            let (sa, sb) = (view.to_screen(p0), view.to_screen(p1));
            draw_line(sa.x, sa.y, sb.x, sb.y, mat.thickness() * view.s, Color::new(col.r, col.g, col.b, 0.6));
            draw_circle(sa.x, sa.y, 5.0, WHITE);
            draw_circle(sb.x, sb.y, 5.0, WHITE);
            let label = format!("{:.0} / {:.0}", len, MEMBER_MAX_LEN);
            ui.text(&label, (sa.x + sb.x) / 2.0 + 8.0, (sa.y + sb.y) / 2.0 - 8.0, 16, WHITE);
        }
        if inside && matches!(self.tool, Tool::Cable(_) | Tool::Spring(_) | Tool::Beam(_)) {
            self.draw_anchor_hint(ui, view, view.to_world(ui.mouse));
        }
        if inside {
            let at = view.to_world(ui.mouse);
            match self.tool {
                Tool::Block(k) => {
                    let cand = self.candidate(k, at);
                    let ok = placement_ok(&self.world, &self.blocks, &cand, None) && self.missing_for_tool(at).is_none();
                    let tint = if ok { Color::new(0.3, 1.0, 0.5, 1.0) } else { Color::new(1.0, 0.1, 0.1, 1.0) };
                    draw_block_shape(k, view.to_screen(cand.pos), view.s, cand.angle, Some(tint), 0.0, 0.0, None);
                    // The studs that would grip: lit up, and counted.
                    let mut grip = 0;
                    for b in &self.blocks {
                        for p in blocks::shared_stud_points(cand.kind, cand.pos, cand.angle, b.kind, b.pos, b.angle) {
                            let q = view.to_screen(p);
                            draw_circle(q.x, q.y, 2.6 * view.s.max(0.9), Color::new(0.4, 1.0, 0.5, 0.95));
                            grip += 1;
                        }
                    }
                    if grip > 0 {
                        ui.text(&format!("{grip} plot{}", if grip > 1 { "s" } else { "" }), ui.mouse.x + 16.0, ui.mouse.y - 10.0, 15, Color::new(0.5, 1.0, 0.6, 1.0));
                    }
                }
                Tool::Mat(_) | Tool::Eraser => {
                    let r = (self.brush.max(if self.tool == Tool::Eraser { 2 } else { 0 }) as f32 + 0.5) * view.s;
                    draw_circle_lines(ui.mouse.x, ui.mouse.y, r, 1.5, Color::new(1.0, 1.0, 1.0, 0.9));
                }
                Tool::Beam(_) => {
                    // Green: it will be pinned there. Orange: no attachment, the rod will fall.
                    if self.snap_point(at).is_none() {
                        draw_circle_lines(ui.mouse.x, ui.mouse.y, 5.0 * view.s, 2.0, ACCENT);
                    }
                }
                Tool::Cable(_) => {
                    if self.snap_point(at).is_none() {
                        // Red: nothing to fix the first end to. Orange: the second end will hang loose.
                        let col = if self.cable_from.is_some() { ACCENT } else { RED };
                        draw_circle_lines(ui.mouse.x, ui.mouse.y, 5.0 * view.s, 2.0, col);
                        if self.cable_from.is_some() {
                            ui.text("bout libre", ui.mouse.x + 10.0, ui.mouse.y + 18.0, 13, ACCENT);
                        }
                    }
                }
                Tool::Shovel => self.draw_shovel_cursor(ui, view, at),
                Tool::Hammer => draw_circle_lines(ui.mouse.x, ui.mouse.y, 3.5 * view.s, 1.5, ACCENT),
                Tool::Loupe => self.draw_lens(ui, view, at),
                _ => {}
            }
        }

        // The loupe tool is its own pointer.
        let loupe = inside && self.tool == Tool::Loupe;
        show_mouse(!loupe);
        if loupe {
            self.draw_lens_cursor(ui);
        }

        // Right at the cursor: building is impossible, the materials are missing.
        if inside {
            if let Some((name, units, have)) = self.missing_for_tool(view.to_world(ui.mouse)) {
                let text = format!("Pas assez de {name} : {have} / {units}");
                let w = ui.text_w(&text, 16) + 22.0;
                let r = Rect::new((ui.mouse.x + 16.0).min(view.visible().x + view.visible().w - w - 4.0), ui.mouse.y + 18.0, w, 26.0);
                rrect(r, 13.0, Color::new(0.85, 0.05, 0.05, 0.95));
                ui.text(&text, r.x + 11.0, r.y + 18.0, 16, WHITE);
                draw_circle_lines(ui.mouse.x, ui.mouse.y, 6.0 * view.s.max(1.0), 3.0, Color::new(1.0, 0.1, 0.1, 1.0));
            }
        }
        self.draw_hud(ui, view);
    }

    /// The sky is not drawn in the temperature view, so mark where the sun (or the
    /// moon) is: the sun is the hottest thing up there and warms the air around it.
    fn draw_sun_on_thermal(&self, view: View, ui: &Ui) {
        let w = &self.world;
        let phase = w.phase();
        if w.sun() > 0.0 {
            let (sx, sy) = w.sun_position();
            let c = view.to_screen(vec2(sx, sy));
            for (r, a) in [(64.0, 0.10), (44.0, 0.18), (28.0, 0.32), (16.0, 0.9)] {
                draw_circle(c.x, c.y, r * view.s.max(0.8), Color::new(1.0, 0.92 - r * 0.002, 0.55, a));
            }
            draw_circle_lines(c.x, c.y, 16.0 * view.s.max(0.8), 2.0, WHITE);
            ui.text("Soleil", c.x + 24.0, c.y - 20.0, 15, WHITE);
        } else {
            let u = ((phase - 0.5) / 0.5).clamp(0.0, 1.0);
            let c = view.to_screen(vec2(W as f32 * (0.08 + 0.84 * u), H as f32 * (0.64 - 0.52 * (std::f32::consts::PI * u).sin())));
            draw_circle(c.x, c.y, 12.0 * view.s.max(0.8), Color::new(0.75, 0.82, 1.0, 0.8));
            ui.text("Lune", c.x + 18.0, c.y - 14.0, 14, Color::new(0.85, 0.9, 1.0, 1.0));
        }
    }

    fn draw_hud(&mut self, ui: &Ui, view: View) {
        let r = view.visible();
        let w = &self.world;
        let night = w.sun() <= 0.0;
        let arrow = if w.wind > 0.08 { "→" } else if w.wind < -0.08 { "←" } else { "·" };
        let text = format!(
            "{}  jour {}  ·  {}  ·  {:.0} °C  ·  vent {} {:.0} km/h",
            w.clock_label(),
            w.day_number(),
            w.season().name(),
            w.surface_temp_c(),
            arrow,
            w.wind.abs() * 40.0
        );
        let tw = ui.text_w(&text, 14) + 38.0;
        let pill = Rect::new(r.x + 8.0, r.y + 8.0, tw, 26.0);
        self.hud_hit = pill;
        rrect(pill, 13.0, Color::new(0.0, 0.0, 0.0, 0.5));
        draw_icon(if night { Icon::Moon } else { Icon::Sun }, vec2(pill.x + 16.0, pill.y + 13.0), 17.0, if night { Color::new(0.85, 0.9, 1.0, 1.0) } else { ACCENT });
        ui.text(&text, pill.x + 30.0, pill.y + 18.0, 14, WHITE);
        // Clicking the season changes it.
        let before = format!("{}  jour {}  ·  ", w.clock_label(), w.day_number());
        let season_r = Rect::new(pill.x + 30.0 + ui.text_w(&before, 14) - 3.0, pill.y + 2.0, ui.text_w(w.season().name(), 14) + 6.0, 22.0);
        if ui.hover(season_r) {
            rrect(season_r, 8.0, Color::new(1.0, 1.0, 1.0, 0.18));
            self.hover_info = Some(("Saison".to_string(), "Clique pour passer à la saison suivante.".to_string()));
        }
        if ui.pressed && !self.quit_modal && season_r.contains(ui.mouse) {
            let all = Season::ALL;
            let next = all[(all.iter().position(|x| *x == self.world.season()).unwrap_or(0) + 1) % all.len()];
            self.world.set_season(next);
            self.say(format!("Saison : {}", next.name()));
        }

        let stats = format!("Arrivées {}   Perdues {}", self.arrived, self.lost);
        let sw = ui.text_w(&stats, 14) + 22.0;
        let sp = Rect::new(r.x + 8.0, r.y + r.h - 32.0, sw, 24.0);
        rrect(sp, 12.0, Color::new(0.0, 0.0, 0.0, 0.5));
        ui.text(&stats, sp.x + 11.0, sp.y + 17.0, 14, WHITE);

        let (label, col) = if self.running { ("SIMULATION", RED) } else { ("CONSTRUCTION", BLUE) };
        let bw = ui.text_w(label, 14) + 32.0;
        let gap = 92.0;
        rrect(Rect::new(r.x + r.w - bw - 8.0 - gap, r.y + 8.0, bw, 26.0), 13.0, Color::new(0.0, 0.0, 0.0, 0.5));
        draw_circle(r.x + r.w - bw + 6.0 - gap, r.y + 21.0, 5.0, col);
        ui.text(label, r.x + r.w - bw + 16.0 - gap, r.y + 26.0, 14, WHITE);
    }

    /// Left edge of the picture: beside the palette when it is pinned, else the whole window.
    fn left(&self) -> f32 {
        if self.panel.pin { PANEL_W } else { 0.0 }
    }

    /// How far the palette is pushed off to the left (0 when fully out).
    fn panel_shift(&self) -> f32 {
        -PANEL_W * (1.0 - self.panel.open)
    }

    /// The cars' launch bar, floating at the top of the picture: one button per model.
    fn draw_launch_bar(&mut self, ui: &mut Ui, sw: f32) {
        let (bw, gap, pad) = (92.0, 6.0, 8.0);
        let n = ModelKind::ALL.len() as f32;
        let total = n * bw + (n - 1.0) * gap + pad * 2.0;
        let l = self.left();
        let bar = Rect::new(l + (sw - l - total) / 2.0, TOP_H + 6.0, total, 50.0);
        self.launch_hit = bar;
        rrect(bar, 16.0, Color::new(0.06, 0.07, 0.13, 0.9));
        for (i, k) in ModelKind::ALL.iter().enumerate() {
            let m = k.model();
            let r = Rect::new(bar.x + pad + i as f32 * (bw + gap), bar.y + 5.0, bw, 40.0);
            let hot = ui.hover(r);
            rrect(r, 10.0, if hot { ui::TILE_HOT } else { ui::TILE });
            let c = self.tunings[i].color.unwrap_or(m.color);
            draw_car_icon(vec2(r.x + 20.0, r.y + 19.0), Color::from_rgba(c.0, c.1, c.2, 255), 0.8);
            ui.text(m.name, r.x + 40.0, r.y + 24.0, 13, TEXT);
            if hot {
                self.hover_info = Some((m.name.to_string(), format!("{} Clique pour lancer la voiture ; ses réglages s'affichent sous les vues, et ceux du modèle sont dans l'onglet Voitures.", m.blurb)));
            }
            if hot && ui.pressed && !self.quit_modal {
                self.model = *k;
                self.launch_car();
                self.car_remote = true;
                self.selected_motor = None;
            }
        }
    }

    fn draw_ui(&mut self, ui: &mut Ui, sw: f32, sh: f32) {
        self.hover_info = None;
        // Under the leave-the-game window nothing answers a click.
        let real_click = ui.pressed;
        if self.quit_modal {
            ui.pressed = false;
        }
        // The palette: it slides out when the mouse goes to the left edge, and tucks away again.
        let handle_w = 16.0;
        let edge = PANEL_W + self.panel_shift();
        let handle = Rect::new(edge.max(0.0), TOP_H, handle_w, sh - TOP_H);
        let hovering = (ui.mouse.x < edge.max(handle_w) && ui.mouse.y > TOP_H) || handle.contains(ui.mouse);
        self.panel.update(hovering, get_frame_time());
        let shift = self.panel_shift();
        let edge = PANEL_W + shift;
        if self.panel.visible() {
            let outer = (ui.mouse, ui.pressed);
            unsafe { get_internal_gl() }.quad_gl.push_model_matrix(Mat4::from_translation(vec3(shift, 0.0, 0.0)));
            ui.mouse.x -= shift;
            draw_rectangle(0.0, TOP_H, PANEL_W, sh - TOP_H, PANEL);
            self.draw_panel(ui, sh);
            self.draw_info_box(ui, sh);
            unsafe { get_internal_gl() }.quad_gl.pop_model_matrix();
            ui.mouse = outer.0;
        }
        // The handle that stays in sight: click it to pin the palette out (or to unpin it).
        let handle = Rect::new(edge.max(0.0), TOP_H, handle_w, sh - TOP_H);
        rrect(handle, 6.0, Color::new(0.1, 0.12, 0.2, if self.panel.open > 0.5 { 0.92 } else { 0.6 }));
        ui.text_center(if self.panel.pin { "‹" } else { "›" }, handle.x + handle_w / 2.0, handle.y + handle.h / 2.0 + 7.0, 22, if self.panel.pin { ACCENT } else { TEXT });
        if ui.pressed && handle.contains(ui.mouse) && !self.quit_modal {
            self.panel.toggle_pin();
        }
        self.panel_hit = if self.panel.visible() { Rect::new(0.0, TOP_H, edge.max(0.0) + handle_w, sh - TOP_H) } else { handle };
        self.draw_tools_strip(ui, sh);
        self.draw_corner_buttons(ui, sw);
        self.draw_launch_bar(ui, sw);
        self.draw_views(ui, sw, sh);
        self.draw_player(ui, sw, sh);
        if self.show_science {
            self.draw_science(ui, sw, sh);
        }
        if self.quit_modal {
            ui.pressed = real_click;
            self.draw_quit_modal(ui, sw, sh);
        }

        if let Some((msg, t)) = &self.toast {
            let a = (*t / 0.5).min(1.0);
            let w = ui.text_w(msg, 20) + 40.0;
            let l = self.left();
            let r = Rect::new(l + (sw - l - w) / 2.0, TOP_H + 64.0, w, 42.0);
            rrect(r, 21.0, Color::new(0.05, 0.06, 0.1, 0.92 * a));
            ui.text(msg, r.x + 20.0, r.y + 27.0, 20, Color::new(1.0, 1.0, 1.0, a));
        }
    }

    /// The player: play/pause and speed, floating at the bottom of the picture like a
    /// video player. It slides away when the mouse has not moved for a moment.
    fn draw_player(&mut self, ui: &mut Ui, sw: f32, sh: f32) {
        let dt = get_frame_time();
        let moved = (ui.mouse - self.last_mouse).length() > 0.5 || is_mouse_button_down(MouseButton::Left) || is_mouse_button_down(MouseButton::Right);
        self.last_mouse = ui.mouse;
        if moved {
            self.idle = 0.0;
        } else {
            self.idle += dt;
        }
        let widths = [108.0f32, 38.0, 46.0, 38.0];
        let gap = 6.0;
        let pad = 12.0;
        let total: f32 = widths.iter().sum::<f32>() + gap * (widths.len() as f32 - 1.0) + pad * 2.0;
        let centre = self.left() + (sw - self.left()) / 2.0;
        let rest = Rect::new(centre - total / 2.0, sh - 66.0, total, 50.0);
        let over = rest.contains(ui.mouse);
        let want = if self.idle < 2.5 || over || self.player_pin || self.quit_modal { 1.0 } else { 0.0 };
        self.player_alpha += (want - self.player_alpha) * (dt * 9.0).min(1.0);
        if (self.player_alpha - want).abs() < 0.01 {
            self.player_alpha = want;
        }
        if self.player_alpha < 0.02 {
            return;
        }
        // It slides down out of sight.
        let bar = Rect::new(rest.x, rest.y + (1.0 - self.player_alpha) * 90.0, rest.w, rest.h);
        draw_rectangle(bar.x - 4.0, bar.y - 4.0, bar.w + 8.0, bar.h + 8.0, Color::new(0.0, 0.0, 0.0, 0.0));
        rrect(Rect::new(bar.x, bar.y + 3.0, bar.w, bar.h), 25.0, Color::new(0.0, 0.0, 0.0, 0.35));
        rrect(bar, 25.0, Color::new(0.07, 0.08, 0.15, 0.94));
        // Buttons answer only while it is (nearly) in place.
        let live = self.player_alpha > 0.6;
        let was_pressed = ui.pressed;
        if !live {
            ui.pressed = false;
        }
        let h = 36.0;
        let y = bar.y + (bar.h - h) / 2.0;
        let mut x = bar.x + pad;
        let mut place = |i: usize, x: &mut f32| -> Rect {
            let r = Rect::new(*x, y, widths[i], h);
            *x += widths[i] + gap;
            r
        };
        let r = place(0, &mut x);
        let (label, icon) = if self.running { ("Pause", Icon::Pause) } else { ("Jouer", Icon::Play) };
        if ui.button(r, label, icon, !self.running, true, GREEN) {
            let run = !self.running;
            self.set_running(run);
        }
        let r = place(1, &mut x);
        if ui.button(r, "", Icon::Minus, false, self.speed > 1, ACCENT) {
            self.speed -= 1;
        }
        let r = place(2, &mut x);
        ui.text_center(&format!("x{}", self.speed), r.x + r.w / 2.0, r.y + h / 2.0 + 6.0, 17, TEXT);
        let r = place(3, &mut x);
        if ui.button(r, "", Icon::Plus, false, self.speed < 6, ACCENT) {
            self.speed += 1;
        }
        ui.pressed = was_pressed;
    }

    fn tile_rect(i: usize, cols: usize, top: f32, h: f32) -> Rect {
        let gap = 6.0;
        let w = (PANEL_W - gap * (cols as f32 + 1.0)) / cols as f32;
        Rect::new(gap + (i % cols) as f32 * (w + gap), top + (i / cols) as f32 * (h + gap), w, h)
    }

    fn draw_panel(&mut self, ui: &mut Ui, sh: f32) {
        // Two rows of tabs: the world, matter and blocks, then mechanics, fluids and cars.
        let tabs = [(Tab::Planet, "Monde"), (Tab::Matter, "Matières"), (Tab::Blocks, "Blocs"), (Tab::Mech, "Méca"), (Tab::Fluid, "Fluides"), (Tab::Cars, "Voitures")];
        let tw = (PANEL_W - 6.0 * 4.0) / 3.0;
        let size = tabs.iter().map(|(_, l)| ui.fit_size(l, tw - 8.0)).min().unwrap_or(16);
        ui.force_size = Some(size);
        for (i, (tab, label)) in tabs.iter().enumerate() {
            let r = Rect::new(6.0 + (i % 3) as f32 * (tw + 6.0), TOP_H + 8.0 + (i / 3) as f32 * 38.0, tw, 32.0);
            if ui.button(r, label, Icon::None, self.tab == *tab, true, ACCENT) {
                self.tab = *tab;
            }
        }
        ui.force_size = None;
        let top = TOP_H + 6.0 + 62.0 + 14.0;
        match self.tab {
            Tab::Matter => self.panel_matter(ui, top),
            Tab::Blocks => self.panel_blocks(ui, top),
            Tab::Cars => self.panel_cars(ui, top),
            Tab::Mech => self.panel_mech(ui, top),
            Tab::Fluid => self.panel_fluid(ui, top),
            Tab::Planet => self.panel_planet(ui, top),
        }
        // Back and undo at the foot of the palette: they are about building.
        let pin_w = 34.0;
        let half = (PANEL_W - 24.0 - pin_w) / 2.0;
        let y = sh - 138.0;
        if ui.button(Rect::new(18.0 + 2.0 * half, y, pin_w, 32.0), "", Icon::Pin, self.panel.pin, true, ACCENT) {
            self.panel.toggle_pin();
        }
        if ui.hover(Rect::new(18.0 + 2.0 * half, y, pin_w, 32.0)) {
            self.hover_info = Some(("Épingler la palette".to_string(), "Épinglée, la palette reste ouverte et l'image se place à côté. Sinon elle se cache et ressort quand la souris touche le bord gauche.".to_string()));
        }
        if ui.button(Rect::new(6.0, y, half, 32.0), "Retour", Icon::Reset, false, true, ACCENT) {
            self.reset();
        }
        let label = if self.undo.is_empty() { "Annuler".to_string() } else { format!("Annuler {}", self.undo.len()) };
        if ui.button(Rect::new(12.0 + half, y, half, 32.0), &label, Icon::Undo, false, !self.undo.is_empty(), BLUE) {
            self.undo();
        }
    }

    /// The tools, always in reach on the left edge of the picture.
    fn draw_tools_strip(&mut self, ui: &mut Ui, sh: f32) {
        let tools: [(Tool, &str, Icon, &str); 6] = [
            (Tool::Select, "Sélection", Icon::Select, "Clique un élément (bloc, tige, engrenage, câble, tuyau, rail, voiture...) : il est entouré. Glisse-le pour le déplacer ; pour une tige, un câble, un tuyau ou un rail, glisse une poignée au bout pour le retailler. Suppr l'efface. Échap désélectionne. Une voiture sélectionnée affiche ses réglages."),
            (Tool::Eraser, "Gomme", Icon::Eraser, "Efface la matière, les blocs et les câbles (clic droit aussi)."),
            (Tool::Hammer, "Marteau", Icon::Hammer, "Tape sur le verre et la glace pour les casser, secoue les autres blocs."),
            (Tool::Loupe, "Loupe", Icon::Loupe, "Le pointeur est une loupe : une vue ronde en bas à droite montre la matière sous la croix."),
            (Tool::Sensor, "Capteur", Icon::Thermo, "Pose une sonde (jusqu'à 4) : thermomètre, baromètre et hygromètre. Ses courbes s'affichent en bas de l'image : l'eau qui chauffe puis reste à 100 °C, une chaudière qui monte en pression, l'air qui sèche à midi. Clique sur une sonde pour l'enlever."),
            (Tool::Shovel, "Pelle", Icon::Shovel, "Clique un tas (cendre, sable, eau, fer...) : la bulle se remplit. Déplace-la, elle reste pleine. Garde le clic enfoncé : plus tu attends, plus le morceau est gros (jusqu'à tout) ; relâche pour le verser. La pelle ne se remplit que si elle est vide. Clic droit : tout verser. Le pinceau règle sa taille."),
        ];
        let (bw, gap) = (38.0, 6.0);
        let total = tools.len() as f32 * (bw + gap) + gap;
        let pill = Rect::new((PANEL_W + self.panel_shift()).max(0.0) + 8.0 + 16.0, (sh - total) / 2.0, bw + 12.0, total);
        self.tools_hit = pill;
        rrect(pill, 14.0, Color::new(0.06, 0.07, 0.13, 0.9));
        for (i, (t, label, icon, text)) in tools.iter().enumerate() {
            let r = Rect::new(pill.x + 6.0, pill.y + gap + i as f32 * (bw + gap), bw, bw);
            if ui.hover(r) {
                self.hover_info = Some((label.to_string(), text.to_string()));
            }
            if ui.button(r, "", *icon, self.tool == *t, true, ACCENT) {
                self.tool = *t;
            }
        }
    }

    /// Science and full screen: two small icons at the top right, above the views.
    fn draw_corner_buttons(&mut self, ui: &mut Ui, sw: f32) {
        let (s, gap) = (34.0, 6.0);
        let sci = Rect::new(sw - 8.0 - s * 2.0 - gap, 10.0, s, s);
        let full = Rect::new(sw - 8.0 - s, 10.0, s, s);
        self.corner_hit = Rect::new(sci.x - 4.0, 4.0, s * 2.0 + gap + 12.0, s + 12.0);
        if ui.hover(sci) {
            self.hover_info = Some(("Science".to_string(), "Les transformations de la matière.".to_string()));
        }
        if ui.hover(full) {
            self.hover_info = Some(("Plein écran".to_string(), "F ou F11 ; Échap en sort.".to_string()));
        }
        if ui.button(sci, "", Icon::Science, self.show_science, true, BLUE) {
            self.show_science = !self.show_science;
            self.science_fresh = self.show_science;
        }
        if ui.button(full, "", Icon::Full, self.fullscreen, true, BLUE) {
            self.toggle_fullscreen();
        }
    }

    fn panel_matter(&mut self, ui: &mut Ui, top: f32) {
        // Brush size.
        ui.text("Pinceau", 12.0, top + 18.0, 16, DIM);
        let minus = Rect::new(86.0, top + 2.0, 32.0, 28.0);
        let plus = Rect::new(PANEL_W - 44.0, top + 2.0, 32.0, 28.0);
        if ui.button(minus, "", Icon::Minus, false, self.brush > 0, ACCENT) {
            self.brush -= 1;
        }
        if ui.button(plus, "", Icon::Plus, false, self.brush < 40, ACCENT) {
            self.brush += 1;
        }
        let bar = Rect::new(124.0, top + 9.0, PANEL_W - 176.0, 14.0);
        rrect(bar, 7.0, Color::new(0.0, 0.0, 0.0, 0.35));
        rrect(Rect::new(bar.x, bar.y, (bar.w * (self.brush as f32 + 1.0) / 41.0).max(14.0), bar.h), 7.0, ACCENT);

        // Add, replace everything, or replace only the material first clicked on.
        let third = (PANEL_W - 24.0) / 3.0;
        for (i, (mode, label)) in [(PaintMode::Add, "Ajouter"), (PaintMode::Replace, "Remplacer"), (PaintMode::Only, "Seulement")].iter().enumerate() {
            let r = Rect::new(6.0 + i as f32 * (third + 6.0), top + 34.0, third, 24.0);
            if ui.hover(r) {
                self.hover_info = Some((
                    label.to_string(),
                    match mode {
                        PaintMode::Add => "Le pinceau remplit le vide ; il ne touche pas à ce qui est déjà là.".into(),
                        PaintMode::Replace => "Le pinceau remplace tout ce qu'il touche, roche, eau, terre...".into(),
                        PaintMode::Only => "Clique sur une matière : seule cette matière est remplacée, en glissant.".into(),
                    },
                ));
            }
            if ui.button(r, label, Icon::None, self.paint_mode == *mode, true, ACCENT) {
                self.paint_mode = *mode;
            }
        }
        let grid_top = top + 34.0 + 30.0;
        // The states of the matter whose arrow was clicked, in a strip under its button. It is
        // drawn over the buttons below, so they must not take its clicks.
        let menu = self.matter_menu.map(|i| {
            let r = Self::tile_rect(i, 3, grid_top, 48.0);
            (i, Rect::new(6.0, r.y + r.h + 2.0, PANEL_W - 12.0, 50.0))
        });
        let pressed = ui.pressed;
        if menu.is_some_and(|(_, mr)| ui.hover(mr)) {
            ui.pressed = false;
        }
        let mut arrow_hit = false;
        for (i, matter) in MATTERS.iter().enumerate() {
            let r = Self::tile_rect(i, 3, grid_top, 48.0);
            let m = matter.states[self.matter_state[i]];
            let inf = info(m);
            let col = mat_color(m);
            let many = matter.states.len() > 1;
            let arrow = Rect::new(r.x + r.w - 25.0, r.y + 4.0, 21.0, 18.0);
            let on_arrow = many && ui.hover(arrow);
            if ui.hover(r) && menu.is_none_or(|(_, mr)| !ui.hover(mr)) {
                let mut fact = inf.fact.to_string();
                if many {
                    let names: Vec<&str> = matter.states.iter().map(|s| info(*s).name).collect();
                    fact = format!("{fact} Ses états : {} (petite flèche du bouton). La chaleur et le froid la font passer de l'un à l'autre.", names.join(", "));
                }
                self.hover_info = Some((format!("{} · {}", inf.name, inf.state), fact));
            }
            let clicked = ui.tile(r, inf.name, self.tool == Tool::Mat(m), |s| {
                rrect(s, 7.0, col);
                rrect(Rect::new(s.x, s.y, s.w, s.h * 0.4), 7.0, Color::new(1.0, 1.0, 1.0, 0.18));
                if matches!(m, Mat::Fire | Mat::Lava | Mat::Molten | Mat::MoltenSteel) {
                    draw_circle(s.x + s.w * 0.5, s.y + s.h * 0.5, s.h * 0.32, Color::new(1.0, 0.9, 0.3, 0.55));
                }
            });
            if many {
                let open = self.matter_menu == Some(i);
                rrect(arrow, 5.0, if open { ACCENT } else if on_arrow { Color::new(0.0, 0.0, 0.0, 0.7) } else { Color::new(0.0, 0.0, 0.0, 0.45) });
                let (c, h) = (vec2(arrow.x + arrow.w / 2.0, arrow.y + arrow.h / 2.0), 4.5);
                let fg = if open { BG } else { TEXT };
                if open {
                    draw_triangle(vec2(c.x - h, c.y + h * 0.5), vec2(c.x + h, c.y + h * 0.5), vec2(c.x, c.y - h * 0.6), fg);
                } else {
                    draw_triangle(vec2(c.x - h, c.y - h * 0.5), vec2(c.x + h, c.y - h * 0.5), vec2(c.x, c.y + h * 0.6), fg);
                }
            }
            if clicked {
                if on_arrow {
                    arrow_hit = true;
                    self.matter_menu = if self.matter_menu == Some(i) { None } else { Some(i) };
                } else {
                    self.tool = Tool::Mat(m);
                }
            }
        }
        ui.pressed = pressed;
        if let Some((i, mr)) = menu {
            let matter = &MATTERS[i];
            rrect(Rect::new(mr.x, mr.y + 2.0, mr.w, mr.h), 10.0, Color::new(0.0, 0.0, 0.0, 0.45));
            rrect(mr, 10.0, PANEL);
            ui::rrect_lines(mr, 10.0, 2.0, ACCENT);
            let n = matter.states.len();
            let w = (mr.w - 6.0 * (n as f32 + 1.0)) / n as f32;
            for (k, st) in matter.states.iter().enumerate() {
                let cr = Rect::new(mr.x + 6.0 + k as f32 * (w + 6.0), mr.y + 6.0, w, mr.h - 12.0);
                let inf = info(*st);
                let hot = ui.hover(cr);
                let chosen = self.matter_state[i] == k;
                rrect(cr, 7.0, if hot { TILE_HOT } else { TILE });
                if chosen {
                    ui::rrect_lines(cr, 7.0, 2.0, ACCENT);
                }
                rrect(Rect::new(cr.x + 5.0, cr.y + 5.0, cr.w - 10.0, 12.0), 4.0, mat_color(*st));
                let size = ui.fit_size(inf.name, cr.w - 4.0).min(14);
                ui.text_center(inf.name, cr.x + cr.w / 2.0, cr.y + cr.h - 6.0, size, if chosen { ACCENT } else { TEXT });
                if hot {
                    self.hover_info = Some((format!("{} · {}", inf.name, inf.state), inf.fact.to_string()));
                    if ui.pressed {
                        self.matter_state[i] = k;
                        self.tool = Tool::Mat(*st);
                        self.matter_menu = None;
                    }
                }
            }
            // A click anywhere else closes the strip.
            if ui.pressed && !arrow_hit && !ui.hover(mr) {
                self.matter_menu = None;
            }
        }
        let rows = MATTERS.len().div_ceil(3);
        self.unlimited_toggles(ui, grid_top + rows as f32 * 54.0 + 2.0);
    }

    /// Vivid red over an element that cannot be built: the materials are missing.
    fn mark_missing(r: Rect) {
        rrect(r, 12.0, Color::new(0.95, 0.08, 0.08, 0.6));
        ui::rrect_lines(r, 12.0, 3.0, Color::new(1.0, 0.15, 0.15, 1.0));
    }

    /// If the selected tool cannot be used for lack of materials: what is missing,
    /// how much it takes and how much there is. `at` is the cursor in world cells.
    fn missing_for_tool(&self, at: Vec2) -> Option<(String, u32, u32)> {
        if self.free_now() {
            return None;
        }
        let drafted = |from: Vec2| (at - from).length();
        let (from, units): (&[Mat], u32) = match self.tool {
            Tool::Block(k) => k.cost(),
            Tool::Cable(m) => {
                let len = self.cable_from.as_ref().and_then(|a| anchor_world(a, &self.blocks, &self.members)).map_or(60.0, drafted);
                (m.cost_from(), m.cost(len * 1.06))
            }
            Tool::Spring(k) => {
                let len = self.spring_draft.as_ref().map_or(60.0, |d| d.rest.map_or_else(|| drafted(d.pa).min(SPRING_MAX_LEN), |r| r.1));
                (k.cost_from(), k.cost(len))
            }
            Tool::Beam(m) => {
                let len = self.member_from.as_ref().map_or(60.0, |f| drafted(f.1).min(MEMBER_MAX_LEN));
                (m.cost_from(), m.cost(len))
            }
            Tool::Gear(_) | Tool::Motor | Tool::Axle | Tool::Rail | Tool::Crank | Tool::Pivot | Tool::Screw | Tool::Winch | Tool::Pulley => self.mech_cost(at)?,
            Tool::Pipe | Tool::Valve | Tool::Pump => self.fluid_cost(at)?,
            _ => return None,
        };
        let have = self.stock(from);
        (have < units).then(|| (info(from[0]).name.to_lowercase(), units, have))
    }

    fn stock(&self, from: &[Mat]) -> u32 {
        from.iter().map(|m| self.census[*m as usize]).sum()
    }

    /// Free building (materials not used up), chosen separately for editing and for the simulation, and
    /// changeable at any time. Shown in the matter and blocks palettes.
    fn unlimited_toggles(&mut self, ui: &mut Ui, y0: f32) {
        let half = (PANEL_W - 18.0) / 2.0;
        ui.text("Matériaux illimités :", 10.0, y0 + 10.0, 13, DIM);
        if ui.checkbox(Rect::new(6.0, y0 + 16.0, half, 24.0), "Édition", self.unlimited, true) {
            self.unlimited = !self.unlimited;
        }
        if ui.checkbox(Rect::new(12.0 + half, y0 + 16.0, half, 24.0), "Simulation", self.unlimited_sim, true) {
            self.unlimited_sim = !self.unlimited_sim;
        }
        if ui.hover(Rect::new(6.0, y0 + 16.0, PANEL_W - 12.0, 24.0)) {
            self.hover_info = Some(("Matériaux illimités".to_string(), "Coché : les blocs, câbles, tiges et mécanismes ne consomment pas la matière du monde. Édition : quand le jeu est à l'arrêt. Simulation : quand il tourne. Décoché : ils prennent la matière (terre, bois, fer, acier, pierre, sable) autour du pointeur.".to_string()));
        }
    }

    fn panel_blocks(&mut self, ui: &mut Ui, top: f32) {
        let angle = self.rot90 as f32 * std::f32::consts::FRAC_PI_2;
        let free = self.free_now();
        for (i, k) in BlockKind::PLACEABLE.iter().enumerate() {
            let r = Self::tile_rect(i, 3, top, 54.0);
            let spec = k.spec();
            let (from, units) = k.cost();
            let have = self.stock(from);
            let short = !free && have < units;
            if ui.hover(r) {
                let cost = format!("Coût : {units} de {}. En stock : {have}.", info(from[0]).name.to_lowercase());
                self.hover_info = Some((spec.name.to_string(), format!("{} {cost}", spec.blurb)));
            }
            if ui.tile(r, spec.name, self.tool == Tool::Block(*k), |s| {
                let sc = (s.w / spec.w.max(spec.h)).min(s.h / spec.w.min(spec.h)).min(1.9);
                draw_block_shape(*k, vec2(s.x + s.w / 2.0, s.y + s.h / 2.0), sc, angle, None, 0.0, 0.0, None);
            }) {
                self.tool = Tool::Block(*k);
            }
            if short {
                Self::mark_missing(r);
            }
        }
        // Cables of three materials: they differ in elasticity, strength and weight.
        let i = BlockKind::PLACEABLE.len();
        for (k, mat) in CableMat::ALL.iter().enumerate() {
            let r = Self::tile_rect(i + k, 3, top, 54.0);
            if ui.hover(r) {
                self.hover_info = Some((
                    format!("Câble {}", mat.name()),
                    "Clique un premier point (bloc, tige, sol ou bord du monde), tire, clique un second point. Il pend en parabole et casse si on tire trop fort.".into(),
                ));
            }
            let (cr, cg, cb) = mat.color();
            let col = Color::from_rgba(cr.max(120), cg.max(120), cb.max(120), 255);
            let label = format!("Câble {}", mat.name());
            let short = !free && self.stock(mat.cost_from()) < mat.cost(60.0);
            if ui.tile(r, &label, self.tool == Tool::Cable(*mat), |s| {
                draw_icon(Icon::Cable, vec2(s.x + s.w / 2.0, s.y + s.h / 2.0), 28.0, col);
            }) {
                self.tool = Tool::Cable(*mat);
            }
            if short {
                Self::mark_missing(r);
            }
        }
        let i = i + CableMat::ALL.len();
        // Springs and elastic cords: three clicks.
        for (k, kind) in SpringKind::ALL.iter().enumerate() {
            let r = Self::tile_rect(i + k, 3, top, 54.0);
            if ui.hover(r) {
                self.hover_info = Some((
                    kind.name().to_string(),
                    "Trois clics : le premier fixe un bout, le deuxième donne la longueur au repos, le troisième attache l'autre bout en le tirant (clique dans le vide pour le laisser libre).".into(),
                ));
            }
            let (cr, cg, cb) = kind.color();
            let short = !free && self.stock(kind.cost_from()) < kind.cost(60.0);
            if ui.tile(r, kind.name(), self.tool == Tool::Spring(*kind), |s| {
                let (a, b) = (vec2(s.x + 8.0, s.y + s.h / 2.0), vec2(s.x + s.w - 8.0, s.y + s.h / 2.0));
                let mut prev = a;
                for j in 1..=12 {
                    let t = j as f32 / 12.0;
                    let q = vec2(a.x + (b.x - a.x) * t, a.y + if j % 2 == 0 { 5.0 } else { -5.0 } * if j == 12 { 0.0 } else { 1.0 });
                    draw_line(prev.x, prev.y, q.x, q.y, 2.0, Color::from_rgba(cr, cg, cb, 255));
                    prev = q;
                }
            }) {
                self.tool = Tool::Spring(*kind);
            }
            if short {
                Self::mark_missing(r);
            }
        }
        let i = i + SpringKind::ALL.len();
        // Free beams: pinned at both ends, like a bridge-building game.
        for (k, mat) in MemberMat::ALL.iter().enumerate() {
            let r = Self::tile_rect(i + k, 3, top, 54.0);
            let (units, from) = (mat.cost(60.0), mat.cost_from());
            if ui.hover(r) {
                self.hover_info = Some((
                    format!("Tige {}", mat.name().to_lowercase()),
                    format!(
                        "Un clic fixe un bout (sol, bloc ou nœud ; sans attache la tige tombe), tire, un second clic pour l'autre bout. Longueur max {:.0}. 60 de long coûte {units} de {}.",
                        MEMBER_MAX_LEN,
                        info(from[0]).name.to_lowercase()
                    ),
                ));
            }
            let (cr, cg, cb) = mat.color();
            let label = format!("Tige {}", mat.name().to_lowercase());
            let short = !free && self.stock(mat.cost_from()) < mat.cost(60.0);
            if ui.tile(r, &label, self.tool == Tool::Beam(*mat), |s| {
                let (a, b) = (vec2(s.x + 10.0, s.y + s.h - 6.0), vec2(s.x + s.w - 10.0, s.y + 6.0));
                draw_line(a.x, a.y, b.x, b.y, 6.0, Color::from_rgba(cr, cg, cb, 255));
                for p in [a, b] {
                    draw_circle(p.x, p.y, 4.5, Color::from_rgba(226, 228, 236, 255));
                    draw_circle(p.x, p.y, 2.0, Color::from_rgba(60, 64, 78, 255));
                }
            }) {
                self.tool = Tool::Beam(*mat);
            }
            if short {
                Self::mark_missing(r);
            }
        }

        // Cable elasticity.
        let rows_used = (i + MemberMat::ALL.len() + 2) / 3;
        let sy = top + rows_used as f32 * 60.0 + 2.0;
        ui.text("Élasticité câble", 10.0, sy + 11.0, 13, DIM);
        ui.slider(Rect::new(110.0, sy, PANEL_W - 150.0, 18.0), &mut self.cable_soft, 0.4, 3.0, ACCENT);
        ui.text(&format!("x{:.1}", self.cable_soft), PANEL_W - 34.0, sy + 11.0, 13, TEXT);

        // Materials: a block uses up what it is made of from the screen.
        let y0 = sy + 26.0;
        self.unlimited_toggles(ui, y0);
        let rows: [(&str, &[Mat]); 6] = [
            ("Terre", &[Mat::Dirt]),
            ("Bois", &[Mat::Wood, Mat::Trunk, Mat::Branch]),
            ("Fer", &[Mat::Iron]),
            ("Acier", &[Mat::Steel]),
            ("Pierre", &[Mat::Stone]),
            ("Sable", &[Mat::Sand]),
        ];
        for (i, (name, mats)) in rows.iter().enumerate() {
            let y = y0 + 46.0 + i as f32 * 19.0;
            let n = self.stock(mats);
            let col = mat_color(mats[0]);
            ui.text(name, 10.0, y + 11.0, 13, DIM);
            let bar = Rect::new(66.0, y + 2.0, PANEL_W - 132.0, 10.0);
            rrect(bar, 6.0, Color::new(0.0, 0.0, 0.0, 0.35));
            rrect(Rect::new(bar.x, bar.y, (bar.w * (n as f32 / 3000.0).min(1.0)).max(if n > 0 { 6.0 } else { 0.0 }), bar.h), 6.0, col);
            ui.text(&format!("{n}"), bar.x + bar.w + 8.0, y + 11.0, 12, TEXT);
        }
        ui.text("Rouge vif = pas assez : peins-en !", 10.0, y0 + 46.0 + 6.0 * 19.0 + 10.0, 12, DIM);
    }

    fn panel_cars(&mut self, ui: &mut Ui, top: f32) {
        // Where the cars start and where they must arrive.
        ui.text("Départ et arrivée", 10.0, top + 14.0, 15, ACCENT);
        let half = (PANEL_W - 18.0) / 2.0;
        if ui.button(Rect::new(6.0, top + 22.0, half, 30.0), "Entrée", Icon::Flag, self.tool == Tool::Entry, true, GREEN) {
            self.tool = if self.tool == Tool::Entry { Tool::Select } else { Tool::Entry };
        }
        if ui.button(Rect::new(12.0 + half, top + 22.0, half, 30.0), "Sortie", Icon::Flag, self.tool == Tool::Exit, true, RED) {
            self.tool = if self.tool == Tool::Exit { Tool::Select } else { Tool::Exit };
        }
        if ui.hover(Rect::new(6.0, top + 22.0, PANEL_W - 12.0, 30.0)) {
            self.hover_info = Some(("Entrée / Sortie".to_string(), "Choisis, puis clique dans le monde : l'entrée est le départ des voitures, la sortie l'arrivée. Reclique le bouton pour revenir à la sélection.".to_string()));
        }
        // Each model's settings: the cars launched from the bar at the top start with these. Once a car is
        // on the track its own settings are in the panel under the views.
        ui.text("Réglages des modèles", 10.0, top + 74.0, 15, ACCENT);
        let tile_w = (PANEL_W - 18.0) / 2.0;
        for (i, k) in ModelKind::ALL.iter().enumerate() {
            let m = k.model();
            let r = Rect::new(6.0 + (i % 2) as f32 * (tile_w + 6.0), top + 82.0 + (i / 2) as f32 * 40.0, tile_w, 36.0);
            let sel = self.model == *k;
            let hot = ui.hover(r);
            rrect(r, 10.0, if hot { ui::TILE_HOT } else { ui::TILE });
            if sel {
                ui::rrect_lines(r, 10.0, 3.0, ACCENT);
            }
            let c = self.tunings[i].color.unwrap_or(m.color);
            let c = Color::from_rgba(c.0, c.1, c.2, 255);
            draw_car_icon(vec2(r.x + 24.0, r.y + 18.0), c, 0.9);
            ui.text(m.name, r.x + 46.0, r.y + 22.0, 15, if sel { ACCENT } else { TEXT });
            if hot {
                self.hover_info = Some((m.name.to_string(), format!("{} Règle ici ce modèle ; il sera lancé avec ces réglages.", m.blurb)));
            }
            if hot && ui.pressed {
                self.model = *k;
                self.selected_car = None;
            }
        }
        let ey = top + 82.0 + 2.0 * 40.0 + 6.0;
        ui.text(&format!("{} : réglages par défaut", self.model.model().name), 10.0, ey + 8.0, 13, DIM);
        self.tuning_editor(ui, 0.0, ey + 16.0, PANEL_W, false);
    }

    /// The settings of the selected car (or of the model being launched), under the views.
    fn draw_car_remote(&mut self, ui: &mut Ui, panel: Rect) {
        rrect(panel, 12.0, Color::new(0.09, 0.1, 0.17, 0.95));
        let editing_live = self.selected_car.map_or(false, |id| self.cars.iter().any(|c| c.id == id));
        let name = self.model.model().name;
        ui.text(&format!("Réglages · {name}"), panel.x + 10.0, panel.y + 20.0, 15, ACCENT);
        ui.text(if editing_live { "voiture en piste" } else { "prochaines voitures" }, panel.x + 10.0, panel.y + 34.0, 11, DIM);
        if ui.button(Rect::new(panel.x + panel.w - 30.0, panel.y + 6.0, 24.0, 22.0), "×", Icon::None, false, true, DIM) {
            self.car_remote = false;
        }
        self.tuning_editor(ui, panel.x, panel.y + 46.0, panel.w, true);
    }

    /// The sliders and colours of a car's settings, from `y` down. `live` edits the selected car on the
    /// track (or the model being launched); otherwise the chosen model's defaults. Returns the bottom.
    fn tuning_editor(&mut self, ui: &mut Ui, x: f32, y0: f32, w: f32, live: bool) -> f32 {
        let rows: [(&str, f32, f32, usize); 8] = [
            ("Puissance", 0.5, 2.0, 0),
            ("Vitesse", 0.5, 2.0, 1),
            ("Suspension", 0.5, 2.0, 2),
            ("Amortiss.", 0.5, 2.0, 3),
            ("Adhérence", 0.6, 1.6, 4),
            ("Roues", 0.7, 1.5, 5),
            ("Solidité", 0.6, 2.0, 6),
            ("Poids", 0.5, 3.0, 7),
        ];
        let mut changed = false;
        for (i, (label, lo, hi, which)) in rows.iter().enumerate() {
            let y = y0 + i as f32 * 23.0;
            let mut t = if live { *self.tuning_mut() } else { self.tunings[self.model_index()] };
            let v = match which {
                0 => &mut t.power,
                1 => &mut t.speed,
                2 => &mut t.stiffness,
                3 => &mut t.damping,
                4 => &mut t.grip,
                5 => &mut t.wheel,
                6 => &mut t.tough,
                _ => &mut t.weight,
            };
            ui.text(label, x + 10.0, y + 12.0, 12, DIM);
            let before = *v;
            ui.slider(Rect::new(x + 82.0, y + 2.0, w - 128.0, 16.0), v, *lo, *hi, ACCENT);
            ui.text(&format!("{:.1}", *v), x + w - 38.0, y + 12.0, 12, TEXT);
            if (*v - before).abs() > 1e-4 {
                if live {
                    *self.tuning_mut() = t;
                } else {
                    let k = self.model_index();
                    self.tunings[k] = t;
                }
                changed = true;
            }
        }
        let cy = y0 + 8.0 * 23.0 + 6.0;
        for (i, c) in Tuning::COLORS.iter().enumerate() {
            let cx = x + 16.0 + i as f32 * 19.0;
            let r = Rect::new(cx - 9.0, cy, 18.0, 18.0);
            let chosen = if live { self.tuning_mut().color } else { self.tunings[self.model_index()].color } == Some(*c);
            draw_circle(cx, cy + 9.0, 7.5, Color::from_rgba(c.0, c.1, c.2, 255));
            if chosen {
                draw_circle_lines(cx, cy + 9.0, 10.0, 2.0, WHITE);
            }
            if ui.hover(r) && ui.pressed {
                if live {
                    self.tuning_mut().color = Some(*c);
                } else {
                    let k = self.model_index();
                    self.tunings[k].color = Some(*c);
                }
                changed = true;
            }
        }
        if ui.button(Rect::new(x + 8.0, cy + 26.0, w - 16.0, 26.0), "Réinitialiser", Icon::Reset, false, true, ACCENT) {
            if live {
                *self.tuning_mut() = Tuning::default();
            } else {
                let k = self.model_index();
                self.tunings[k] = Tuning::default();
            }
            changed = true;
        }
        if changed && live {
            self.sync_default_tuning();
        }
        cy + 52.0
    }

    fn panel_planet(&mut self, ui: &mut Ui, top: f32) {
        ui.text("Nouveau monde", 10.0, top + 14.0, 16, ACCENT);
        for (i, b) in Biome::ALL.iter().enumerate() {
            let r = Rect::new(6.0, top + 22.0 + i as f32 * 32.0, PANEL_W - 12.0, 28.0);
            if ui.button(r, b.name(), Icon::World, self.world.biome == *b, true, GREEN) {
                self.new_world(*b);
            }
        }
        // A world with nothing in it, to build from scratch.
        if ui.button(Rect::new(6.0, top + 22.0 + 3.0 * 32.0, PANEL_W - 12.0, 28.0), "Monde vide", Icon::Blank, false, true, ACCENT) {
            self.empty_world();
        }
        // Saving: three slots, then save or load into the chosen one.
        let sy0 = top + 22.0 + 4.0 * 32.0 + 12.0;
        ui.text("Sauvegarde", 10.0, sy0, 16, ACCENT);
        let slot_w = (PANEL_W - 24.0) / 3.0;
        for i in 0..3 {
            let used = save::slot_used(i);
            let r = Rect::new(6.0 + i as f32 * (slot_w + 6.0), sy0 + 6.0, slot_w, 28.0);
            if ui.button(r, &format!("{}", i + 1), Icon::None, self.slot == i, true, if used { ACCENT } else { DIM }) {
                self.slot = i;
            }
            if used && self.slot != i {
                draw_circle(r.x + r.w - 6.0, r.y + 6.0, 3.5, ACCENT);
            }
        }
        let half = (PANEL_W - 18.0) / 2.0;
        if ui.button(Rect::new(6.0, sy0 + 40.0, half, 28.0), "Sauver", Icon::Save, false, true, ACCENT) {
            self.save();
        }
        if ui.button(Rect::new(12.0 + half, sy0 + 40.0, half, 28.0), "Charger", Icon::Load, false, save::slot_used(self.slot), ACCENT) {
            self.load();
        }
        let ty = sy0 + 82.0;
        ui.text("Temps", 10.0, ty, 16, ACCENT);
        let half = (PANEL_W - 18.0) / 2.0;
        if ui.button(Rect::new(6.0, ty + 6.0, half, 28.0), "Midi", Icon::Sun, false, true, ACCENT) {
            self.world.set_phase(0.25);
        }
        if ui.button(Rect::new(12.0 + half, ty + 6.0, half, 28.0), "Minuit", Icon::Moon, false, true, BLUE) {
            self.world.set_phase(0.75);
        }
        let cycle = self.world.day_cycle;
        if ui.button(Rect::new(6.0, ty + 40.0, PANEL_W - 12.0, 26.0), if cycle { "Jour et nuit : en marche" } else { "Jour et nuit : figé" }, Icon::None, cycle, true, GREEN) {
            self.world.day_cycle = !cycle;
        }
        let sy = ty + 80.0;
        ui.text("Saison", 10.0, sy, 16, ACCENT);
        for (i, s) in Season::ALL.iter().enumerate() {
            let r = Rect::new(6.0 + (i % 2) as f32 * (half + 6.0), sy + 6.0 + (i / 2) as f32 * 32.0, half, 27.0);
            if ui.button(r, s.name(), Icon::None, self.world.season() == *s, true, ACCENT) {
                self.world.set_season(*s);
            }
        }
        let iy = sy + 6.0 + 2.0 * 32.0 + 14.0;
        let w = &self.world;
        ui.text(&format!("Air {:.0} °C · vent {:.0} km/h {}", w.surface_temp_c(), w.wind.abs() * 40.0, if w.wind > 0.0 { "→" } else { "←" }), 10.0, iy, 14, TEXT);
        ui.text("Vent", 10.0, iy + 22.0, 16, ACCENT);
        let (maxv, ch, windy) = (self.world.wind_max, self.world.wind_changes, self.world.windiness);
        let rows: [(&str, f32, f32, f32, String); 3] = [
            ("Force max", maxv, 0.05, 1.0, format!("{:.0} km/h", maxv * 40.0)),
            ("Virages", ch, 0.0, 3.0, if ch < 0.05 { "jamais".to_string() } else { format!("x{ch:.1}") }),
            ("Calme-venteux", windy, 0.0, 1.0, format!("{:.0} %", windy * 100.0)),
        ];
        let mut vals = [maxv, ch, windy];
        for (k, (label, _, lo, hi, shown)) in rows.iter().enumerate() {
            let y = iy + 30.0 + k as f32 * 22.0;
            ui.text(label, 10.0, y + 12.0, 12, TEXT);
            ui.slider(Rect::new(96.0, y, PANEL_W - 96.0 - 74.0, 18.0), &mut vals[k], *lo, *hi, BLUE);
            ui.text(shown, PANEL_W - 64.0, y + 13.0, 12, DIM);
        }
        self.world.wind_max = vals[0];
        self.world.wind_changes = vals[1];
        self.world.windiness = vals[2];
        let cr = Rect::new(10.0, iy + 30.0 + 3.0 * 22.0 + 2.0, PANEL_W - 20.0, 24.0);
        if ui.checkbox(cr, "Air compressible (P V = n R T)", self.world.flow.compress, true) {
            self.world.flow.compress = !self.world.flow.compress;
        }
    }

    /// The palette of views on the right edge: it slides out while the mouse is over it (or at the
    /// edge), tucks away when the mouse leaves, and stays out when pinned, like the left palette.
    fn draw_views(&mut self, ui: &mut Ui, sw: f32, sh: f32) {
        let y = TOP_H + 60.0;
        let h = 376.0;
        let tab = Rect::new(sw - VIEWS_TAB, y, VIEWS_TAB, 96.0);
        let px = sw - VIEWS_TAB - VIEWS_W * self.views.open;
        let panel = Rect::new(px, y, VIEWS_W, h);
        let near_edge = ui.mouse.x > sw - 8.0 && ui.mouse.y > TOP_H;
        let hovering = tab.contains(ui.mouse) || (self.views.open > 0.05 && panel.contains(ui.mouse)) || near_edge;
        self.views.update(hovering, get_frame_time());
        let px = sw - VIEWS_TAB - VIEWS_W * self.views.open;
        let panel = Rect::new(px, y, VIEWS_W, h);
        // The motor's remote control and the car settings are not part of the menu: they stay in
        // sight, where the menu's foot would be, as long as a motor or a car is chosen.
        let rest = Rect::new(sw - VIEWS_TAB - VIEWS_W, y, VIEWS_W, h);
        let remote = self.remote_rect(rest);
        let car_panel = if remote.is_none() && self.car_remote { Some(Rect::new(rest.x, rest.y + rest.h + 8.0, rest.w, 286.0)) } else { None };
        let menu = if self.views.visible() { tab.combine_with(panel) } else { tab };
        self.views_hit = match remote.or(car_panel) {
            Some(r) => menu.combine_with(r),
            None => menu,
        };
        if let Some(r) = remote {
            self.draw_remote(ui, r);
        } else if let Some(r) = car_panel {
            self.draw_car_remote(ui, r);
        }

        // The tab that stays visible: click it to pin the views out (or to unpin them).
        rrect(tab, 8.0, Color::new(0.1, 0.12, 0.2, 0.92));
        draw_icon(Icon::Eye, vec2(tab.x + tab.w / 2.0, tab.y + 22.0), 15.0, if self.mode == ViewMode::Normal { DIM } else { ACCENT });
        let arrow = if self.views.pin { "›" } else { "‹" };
        ui.text_center(arrow, tab.x + tab.w / 2.0, tab.y + 56.0, 20, if self.views.pin { ACCENT } else { TEXT });
        if self.mode != ViewMode::Normal {
            draw_circle(tab.x + tab.w / 2.0, tab.y + 78.0, 4.0, ACCENT);
        }
        if ui.pressed && tab.contains(ui.mouse) && !self.quit_modal {
            self.views.toggle_pin();
        }
        if !self.views.visible() {
            return;
        }

        rrect(panel, 12.0, Color::new(0.09, 0.1, 0.17, 0.95));
        ui.text("Vues", panel.x + 12.0, panel.y + 22.0, 16, ACCENT);
        let pin = Rect::new(panel.x + panel.w - 34.0, panel.y + 5.0, 28.0, 24.0);
        if ui.button(pin, "", Icon::Pin, self.views.pin, true, ACCENT) {
            self.views.toggle_pin();
        }
        if ui.hover(pin) {
            self.hover_info = Some(("Épingler les vues".to_string(), "Épinglées, les vues restent ouvertes. Sinon elles se cachent quand la souris s'en va et ressortent quand elle touche le bord droit.".to_string()));
        }
        let modes: [(ViewMode, &str, Icon, Color); 6] = [
            (ViewMode::Normal, "Normale", Icon::Sun, ACCENT),
            (ViewMode::Thermal, "Température", Icon::Thermo, RED),
            (ViewMode::Stress, "Efforts", Icon::Gauge, ACCENT),
            (ViewMode::Wind, "Vent", Icon::Wind, BLUE),
            (ViewMode::Pressure, "Pression", Icon::Gauge, GREEN),
            (ViewMode::Humidity, "Hygrométrie", Icon::Drop, BLUE),
        ];
        for (i, (m, label, icon, col)) in modes.iter().enumerate() {
            let r = Rect::new(panel.x + 8.0, panel.y + 32.0 + i as f32 * 31.0, panel.w - 16.0, 27.0);
            if ui.button(r, label, *icon, self.mode == *m, true, *col) {
                self.mode = *m;
            }
        }
        // Legend for the chosen view.
        let ly = panel.y + 32.0 + 6.0 * 31.0 + 8.0;
        let bar = Rect::new(panel.x + 12.0, ly + 14.0, panel.w - 24.0, 12.0);
        match self.mode {
            ViewMode::Thermal => {
                let n = 50;
                for i in 0..n {
                    let t = -40.0 + 1040.0 * (i as f32 / n as f32).powf(1.6);
                    let (r2, g2, b2) = therm(t);
                    draw_rectangle(bar.x + bar.w * i as f32 / n as f32, bar.y, bar.w / n as f32 + 1.0, bar.h, Color::from_rgba(r2, g2, b2, 255));
                }
                for (t, label) in [(-40.0f32, "-40"), (0.0, "0"), (100.0, "100"), (600.0, "600")] {
                    let u = ((t + 40.0) / 1040.0).powf(1.0 / 1.6);
                    ui.text(label, bar.x + bar.w * u - 5.0, bar.y + 25.0, 11, DIM);
                }
                ui.text("°C", bar.x + bar.w - 14.0, ly + 10.0, 11, DIM);
            }
            ViewMode::Stress => {
                for i in 0..40 {
                    let (r2, g2, b2) = render::stress_rgb(i as f32 / 39.0);
                    draw_rectangle(bar.x + bar.w * i as f32 / 40.0, bar.y, bar.w / 40.0 + 1.0, bar.h, Color::new(r2, g2, b2, 1.0));
                }
                ui.text("détendu", bar.x, bar.y + 25.0, 11, DIM);
                ui.text("casse", bar.x + bar.w - 28.0, bar.y + 25.0, 11, DIM);
                ui.wrapped("Rouge = chargé. Mécanique : anneau = dents, arc = couple, F = force, % = charge moteur.", bar.x, bar.y + 42.0, bar.w, 11, DIM);
            }
            ViewMode::Wind => {
                let w = &self.world;
                ui.text(&format!("{:.0} km/h {}", w.wind.abs() * 40.0, if w.wind > 0.0 { "→" } else { "←" }), bar.x, bar.y + 4.0, 14, TEXT);
                ui.wrapped("Couches de vent opposées et tourbillons : ils portent nuages, vapeur, fumée et pluie, et contournent le relief.", bar.x, bar.y + 22.0, bar.w, 11, DIM);
            }
            ViewMode::Pressure => {
                for i in 0..40 {
                    let atm = 0.35 + 6.0 * (i as f32 / 39.0).powf(2.0);
                    let (r2, g2, b2) = world::pressure_rgb(atm);
                    draw_rectangle(bar.x + bar.w * i as f32 / 40.0, bar.y, bar.w / 40.0 + 1.0, bar.h, Color::from_rgba(r2, g2, b2, 255));
                }
                for (atm, label) in [(0.5f32, "0,5"), (1.0, "1"), (2.0, "2"), (5.0, "5")] {
                    let u = ((atm - 0.35) / 6.0).sqrt();
                    ui.text(label, bar.x + bar.w * u - 4.0, bar.y + 25.0, 11, DIM);
                }
                ui.text("atm", bar.x + bar.w - 20.0, ly + 10.0, 11, DIM);
                ui.wrapped("Air : plus mince en altitude, plus dense dans les courants. Liquides : le poids de la colonne. Solides : le poids au-dessus. Traits sombres : frontières gaz / liquide / solide.", bar.x, bar.y + 42.0, bar.w, 11, DIM);
            }
            ViewMode::Humidity => {
                for i in 0..40 {
                    let f = i as f32 / 39.0;
                    let t = f.clamp(0.0, 1.0);
                    let stops = [(0.0f32, [186.0f32, 134.0, 66.0]), (0.35, [214.0, 196.0, 96.0]), (0.65, [90.0, 180.0, 140.0]), (1.0, [40.0, 100.0, 235.0])];
                    let k = stops.iter().position(|s| s.0 >= t).unwrap_or(3).max(1);
                    let ((p0, c0), (p1, c1)) = (stops[k - 1], stops[k]);
                    let u = ((t - p0) / (p1 - p0)).clamp(0.0, 1.0);
                    draw_rectangle(bar.x + bar.w * f * 0.975, bar.y, bar.w / 40.0 + 1.0, bar.h, Color::from_rgba((c0[0] + (c1[0] - c0[0]) * u) as u8, (c0[1] + (c1[1] - c0[1]) * u) as u8, (c0[2] + (c1[2] - c0[2]) * u) as u8, 255));
                }
                ui.text("sec", bar.x, bar.y + 25.0, 11, DIM);
                ui.text("100 %", bar.x + bar.w - 28.0, bar.y + 25.0, 11, DIM);
                ui.wrapped("Air : part de vapeur d'eau par rapport à ce que l'air peut tenir (l'air froid en tient peu). Liquides : toujours 100 %. Gris : solide sec ; la terre garde son humidité.", bar.x, bar.y + 42.0, bar.w, 11, DIM);
            }
            ViewMode::Normal => {
                ui.wrapped("Touches : H température, X efforts, V vent, P pression, Y hygrométrie.", bar.x, bar.y + 2.0, bar.w, 11, DIM);
            }
        }
        let _ = sh;
    }

    /// Arrows showing the flow of the air, vortices and opposing layers included, where it blows.
    fn draw_wind_field(&self, view: View) {
        let flow = &self.world.flow;
        let step = 3;
        for j in (1..world::flow::FH).step_by(step) {
            for i in (0..world::flow::FW).step_by(step) {
                if flow.is_solid(i, j) {
                    continue;
                }
                let (vx, vy) = flow.at_cell(i, j);
                let speed = (vx * vx + vy * vy).sqrt();
                let centre = vec2((i as f32 + 0.5) * world::flow::CELL as f32, (j as f32 + 0.5) * world::flow::CELL as f32);
                let c = view.to_screen(centre);
                let dir = if speed > 1e-4 { vec2(vx, vy) / speed } else { vec2(1.0, 0.0) };
                let len = (4.0 + 20.0 * speed) * view.s.max(0.8) * 1.4;
                let (a, b) = (c - dir * len * 0.5, c + dir * len * 0.5);
                let col = Color::new(1.0, 1.0, 1.0, (0.2 + 0.6 * (speed / 0.8)).min(0.85));
                draw_line(a.x, a.y, b.x, b.y, 1.8, col);
                let side = vec2(-dir.y, dir.x);
                draw_triangle(b, b - dir * 6.0 + side * 3.6, b - dir * 6.0 - side * 3.6, col);
            }
        }
    }

    /// "Leave the game?": left and right pick a button, Enter validates it, Esc goes back.
    fn draw_quit_modal(&mut self, ui: &mut Ui, sw: f32, sh: f32) {
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.05, 0.62));
        let w = 420.0;
        let r = Rect::new((sw - w) / 2.0, sh / 2.0 - 90.0, w, 180.0);
        rrect(r, 16.0, Color::new(0.09, 0.1, 0.17, 0.98));
        ui::rrect_lines(r, 16.0, 2.0, ACCENT);
        ui.text_center("Quitter AWorld ?", r.x + w / 2.0, r.y + 46.0, 26, ACCENT);
        ui.text_center("La simulation est en pause. ← → choisir · Entrée valider", r.x + w / 2.0, r.y + 76.0, 14, DIM);
        let (bw, by) = ((w - 48.0) / 2.0, r.y + 108.0);
        if ui.button(Rect::new(r.x + 16.0, by, bw, 44.0), "Reprendre (Échap)", Icon::Play, !self.quit_choice, true, GREEN) {
            self.close_quit_modal();
        }
        if ui.button(Rect::new(r.x + 32.0 + bw, by, bw, 44.0), "Quitter", Icon::None, self.quit_choice, true, RED) {
            Self::quit_game();
        }
    }

    fn draw_info_box(&mut self, ui: &Ui, sh: f32) {
        let r = Rect::new(6.0, sh - 100.0, PANEL_W - 12.0, 94.0);
        rrect(r, 10.0, Color::new(0.0, 0.0, 0.0, 0.28));
        match &self.hover_info {
            Some((title, text)) => {
                ui.text(title, r.x + 8.0, r.y + 17.0, 15, ACCENT);
                ui.wrapped(text, r.x + 8.0, r.y + 33.0, r.w - 16.0, 13, TEXT);
            }
            None => {
                ui.text("Espace : jouer / pause", r.x + 8.0, r.y + 19.0, 13, DIM);
                ui.text("R retour · C voiture · Z annuler", r.x + 8.0, r.y + 36.0, 13, DIM);
                ui.text("H thermique · X efforts · V vent", r.x + 8.0, r.y + 53.0, 13, DIM);
                ui.text("Ctrl + molette/glisser : zoom", r.x + 8.0, r.y + 70.0, 13, DIM);
                ui.text("Survole une case pour apprendre", r.x + 8.0, r.y + 90.0, 13, DIM);
            }
        }
    }

    fn draw_science(&mut self, ui: &mut Ui, sw: f32, sh: f32) {
        draw_rectangle(0.0, TOP_H, sw, sh - TOP_H, Color::new(0.02, 0.03, 0.07, 0.86));
        let area = Rect::new(30.0, TOP_H + 20.0, sw - 60.0, sh - TOP_H - 40.0);
        ui.text("Les transformations de la matière", area.x, area.y + 28.0, 28, ACCENT);
        if ui.button(Rect::new(area.x + area.w - 110.0, area.y, 110.0, 36.0), "Fermer", Icon::None, false, true, ACCENT) {
            self.show_science = false;
        }
        // The page is longer than the window: it scrolls by whole rows (wheel, PageUp/PageDown, or the bar
        // on the right), so that no row is ever cut in half.
        let top = area.y + 52.0;
        let view_h = (area.y + area.h - top).max(72.0);
        let col_w = (area.w - 16.0) / 2.0;
        let per_col = RECIPES.len().div_ceil(2);
        ui.text("Molette, PageHaut/PageBas ou barre à droite : faire défiler", area.x + 460.0, area.y + 26.0, 14, DIM);
        let shown = ((view_h - 6.0) / 72.0).floor().max(1.0) as usize;
        let max_row = per_col.saturating_sub(shown);
        let content_h = per_col as f32 * 72.0;
        let mut first = self.science_scroll.round().clamp(0.0, max_row as f32);
        first -= mouse_wheel().1.signum() * if mouse_wheel().1 != 0.0 { 1.0 } else { 0.0 };
        if is_key_pressed(KeyCode::PageDown) {
            first += shown as f32 - 1.0;
        }
        if is_key_pressed(KeyCode::PageUp) {
            first -= shown as f32 - 1.0;
        }
        let track = Rect::new(area.x + area.w - 10.0, top, 8.0, view_h);
        if max_row > 0 && is_mouse_button_down(MouseButton::Left) && Rect::new(track.x - 8.0, track.y, track.w + 16.0, track.h).contains(ui.mouse) {
            first = ((ui.mouse.y - track.y) / track.h * (max_row as f32 + 1.0)).floor();
        }
        first = first.clamp(0.0, max_row as f32);
        self.science_scroll = first;
        let (first, max_scroll) = (first as usize, max_row as f32);
        let scroll = first as f32;
        for (i, rec) in RECIPES.iter().enumerate() {
            let (col, row) = (i / per_col, i % per_col);
            if row < first || row >= first + shown {
                continue;
            }
            let x = area.x + col as f32 * col_w;
            let y = top + 6.0 + (row - first) as f32 * 72.0;
            let mut cx = x;
            let sq = 26.0;
            let swatch = |m: Mat, cx: &mut f32| {
                let c = mat_color(m);
                rrect(Rect::new(*cx, y, sq, sq), 6.0, c);
                ui.text_center(info(m).name, *cx + sq / 2.0, y + sq + 13.0, 11, DIM);
                *cx += sq + 20.0;
            };
            for (j, m) in rec.from.iter().enumerate() {
                if j > 0 {
                    ui.text("+", cx - 16.0, y + 19.0, 20, TEXT);
                }
                swatch(*m, &mut cx);
            }
            ui.text("→", cx - 14.0, y + 20.0, 22, ACCENT);
            cx += 14.0;
            for (j, m) in rec.to.iter().enumerate() {
                if j > 0 {
                    ui.text("+", cx - 16.0, y + 19.0, 20, TEXT);
                }
                swatch(*m, &mut cx);
            }
            ui.wrapped(rec.text, x, y + sq + 32.0, col_w - 30.0, 14, TEXT);
        }
        let _ = content_h;
        if max_scroll > 0.0 {
            rrect(track, 4.0, Color::new(1.0, 1.0, 1.0, 0.12));
            let thumb_h = (shown as f32 / per_col as f32 * track.h).max(24.0);
            let thumb_y = track.y + (track.h - thumb_h) * (scroll / max_scroll);
            rrect(Rect::new(track.x, thumb_y, track.w, thumb_h), 4.0, ACCENT);
        }
        // A click outside closes it, but not the very click that opened it.
        if std::mem::take(&mut self.science_fresh) {
            return;
        }
        if ui.pressed && !area.contains(ui.mouse) {
            self.show_science = false;
        }
    }
}

fn ctrl_down() -> bool {
    is_key_down(KeyCode::LeftControl) || is_key_down(KeyCode::RightControl)
}

fn therm(t: f32) -> (u8, u8, u8) {
    const STOPS: [(f32, [f32; 3]); 8] = [
        (-40.0, [20.0, 30.0, 120.0]),
        (0.0, [40.0, 110.0, 230.0]),
        (20.0, [60.0, 200.0, 180.0]),
        (60.0, [110.0, 220.0, 80.0]),
        (100.0, [240.0, 230.0, 60.0]),
        (300.0, [250.0, 140.0, 30.0]),
        (600.0, [240.0, 50.0, 30.0]),
        (1000.0, [255.0, 255.0, 255.0]),
    ];
    let t = t.clamp(-40.0, 1000.0);
    let k = STOPS.iter().position(|s| s.0 >= t).unwrap_or(7).max(1);
    let (t0, c0) = STOPS[k - 1];
    let (t1, c1) = STOPS[k];
    let f = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
    let m = |a: f32, b: f32| (a + (b - a) * f) as u8;
    (m(c0[0], c1[0]), m(c0[1], c1[1]), m(c0[2], c1[2]))
}

fn off_rot(a: f32, v: Vec2) -> Vec2 {
    let (s, c) = a.sin_cos();
    vec2(v.x * c - v.y * s, v.x * s + v.y * c)
}

fn ground_y(world: &World, x: i32) -> i32 {
    (0..H as i32).find(|&y| world.raw(x, y).is_hard()).unwrap_or(H as i32 - 1)
}

/// Builds a little scene for `--demo`: a bridge over a pit, a tower, a wheel.
fn demo_scene(g: &mut Game, biome: Biome) {
    g.seed = 2;
    let (e, x) = g.world.generate_biome(2, biome);
    g.entry = Some(e);
    g.exit = Some(x);
    g.fresh();
    let gx = 450;
    let gy = ground_y(&g.world, gx);
    let mut z = |kind: BlockKind, x: f32, y: f32| {
        g.blocks.push(Block::new(kind, vec2(x, y), 0.0));
    };
    z(BlockKind::Brick, gx as f32, gy as f32 - 5.0);
    z(BlockKind::Brick, gx as f32, gy as f32 - 15.0);
    z(BlockKind::Long, gx as f32 + 10.0, gy as f32 - 25.0);
    z(BlockKind::Cube, gx as f32 + 60.0, gy as f32 - 10.0);
    z(BlockKind::Glass, gx as f32 + 100.0, gy as f32 - 5.0);
    z(BlockKind::SteelBeam, gx as f32 + 150.0, gy as f32 - 30.0);
    z(BlockKind::Wheel, gx as f32 + 110.0, gy as f32 - 60.0);
    let (a, b) = (g.blocks[0].uid, g.blocks[3].uid);
    let mut cable = Cable::new(Anchor::Block { uid: a, local: vec2(0.0, -5.0) }, Anchor::Block { uid: b, local: vec2(0.0, -10.0) }, 60.0);
    cable.len = 66.0;
    g.cables.push(cable);
    // A small truss: two beams from the ground meeting at a node.
    let (pa, pb) = (vec2(gx as f32 - 90.0, ground_y(&g.world, gx - 90) as f32), vec2(gx as f32 - 20.0, ground_y(&g.world, gx - 20) as f32));
    let apex = vec2(gx as f32 - 55.0, pa.y.min(pb.y) - 50.0);
    let m1 = Member::new(MemberMat::Steel, Anchor::Ground(pa), Anchor::Free(apex), pa, apex);
    let m2 = Member::new(MemberMat::Wood, Anchor::Ground(pb), Anchor::End { uid: m1.uid, end: 1 }, pb, apex);
    g.members.push(m1);
    g.members.push(m2);
    g.world.paint(300, 90, 12, Mat::Cloud);
    g.world.paint(650, gy - 30, 6, Mat::Lava);
    g.world.paint(300, gy - 60, 8, Mat::Water);
    g.world.paint(780, gy - 15, 4, Mat::Fire);
    g.refresh_mask();
    g.model = ModelKind::Offroad;
    g.launch_car();
}

/// `--demo --bridge`: a gap in a flat floor, crossed by a steel deck held up by a small truss.
fn demo_bridge(g: &mut Game) {
    g.empty_world();
    for x in 0..W as i32 {
        if (430..530).contains(&x) {
            continue;
        }
        for y in 330..H as i32 {
            g.world.set(x, y, Mat::Stone);
        }
    }
    let (l, r, mid, low) = (vec2(426.0, 330.0), vec2(534.0, 330.0), vec2(480.0, 330.0), vec2(480.0, 366.0));
    let (dl, dr) = (vec2(430.0, 336.0), vec2(530.0, 336.0));
    let left = Member::new(MemberMat::Steel, Anchor::Ground(l), Anchor::Free(mid), l, mid);
    let right = Member::new(MemberMat::Steel, Anchor::Ground(r), Anchor::End { uid: left.uid, end: 1 }, r, mid);
    let d1 = Member::new(MemberMat::Steel, Anchor::Ground(dl), Anchor::Free(low), dl, low);
    let d2 = Member::new(MemberMat::Steel, Anchor::Ground(dr), Anchor::End { uid: d1.uid, end: 1 }, dr, low);
    let post = Member::new(MemberMat::Steel, Anchor::End { uid: d1.uid, end: 1 }, Anchor::End { uid: left.uid, end: 1 }, low, mid);
    g.members.extend([left, right, d1, d2, post]);
    g.entry = Some((330, 306));
    g.exit = Some((900, 306));
    g.refresh_mask();
    g.model = ModelKind::Truck;
    g.set_running(true);
}

/// A little mechanism for `--demo --mech`: a motor driving a gear train, a crank, a rail and an axle.
fn demo_mech(g: &mut Game) {
    let gx = 450.0;
    let gy = ground_y(&g.world, 450) as f32;
    let mut click = |g: &mut Game, tool: Tool, x: f32, y: f32| {
        g.tool = tool;
        g.mech_click(vec2(x, y));
    };
    click(g, Tool::Motor, gx - 60.0, gy - 12.0);
    click(g, Tool::Gear(GearSize::Large), gx - 40.0, gy - 12.0 - 8.0 * 0.0);
    click(g, Tool::Gear(GearSize::Medium), gx - 40.0 + 24.0, gy - 12.0 - 2.0);
    click(g, Tool::Rail, gx - 20.0, gy - 70.0);
    click(g, Tool::Rail, gx + 100.0, gy - 70.0);
    click(g, Tool::Crank, gx - 40.0 + 24.0 + 8.0, gy - 14.0);
    click(g, Tool::Crank, gx + 40.0, gy - 70.0);
    g.mech_draft = None;
    g.tool = Tool::Gear(GearSize::Medium);
}

/// `--demo --pipes`: two vessels joined by a siphon, with a valve and a motor to turn it.
fn demo_pipes(g: &mut Game) {
    g.empty_world();
    g.world.structures_fall = false;
    for x in 0..W as i32 {
        for y in 330..360 {
            g.world.set(x, y, Mat::Stone);
        }
    }
    for (x0, x1, top) in [(220, 340, 250), (520, 640, 315)] {
        for y in 215..330 {
            for x in x0 - 8..x0 {
                g.world.set(x, y, Mat::Stone);
            }
            for x in x1..x1 + 8 {
                g.world.set(x, y, Mat::Stone);
            }
        }
        for y in top..330 {
            for x in x0..x1 {
                g.world.set(x, y, Mat::Water);
            }
        }
    }
    let was = g.unlimited;
    g.unlimited = true;
    g.tool = Tool::Pipe;
    for (x, y) in [(300.0, 320.0), (300.0, 200.0), (580.0, 200.0), (580.0, 320.0)] {
        g.fluid_click(vec2(x, y));
    }
    g.pipe_from = None;
    g.tool = Tool::Valve;
    g.fluid_click(vec2(440.0, 200.0));
    g.tool = Tool::Motor;
    g.mech_click(vec2(440.0, 150.0));
    g.tool = Tool::Axle;
    g.mech_click(vec2(440.0, 150.0));
    g.mech_click(vec2(440.0, 200.0));
    g.unlimited = was;
    g.tool = Tool::Pipe;
}

/// A little car, `s` times its natural size, centred on `c`.
fn draw_car_icon(c: Vec2, col: Color, s: f32) {
    rrect(Rect::new(c.x - 17.0 * s, c.y - 2.0 * s, 34.0 * s, 10.0 * s), 3.0, col);
    rrect(Rect::new(c.x - 11.0 * s, c.y - 10.0 * s, 18.0 * s, 10.0 * s), 3.0, col);
    draw_circle(c.x - 10.0 * s, c.y + 9.0 * s, 5.0 * s, Color::from_rgba(26, 26, 32, 255));
    draw_circle(c.x + 10.0 * s, c.y + 9.0 * s, 5.0 * s, Color::from_rgba(26, 26, 32, 255));
}

#[macroquad::main(window_conf)]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let shot = arg("--demo");

    let font = load_ttf_font_from_bytes(include_bytes!("../assets/font.ttf")).expect("font");
    let mut ui = Ui { font, mouse: Vec2::ZERO, pressed: false, force_size: None };
    let mut renderer = Renderer::new();
    let mut game = Game::new();
    #[cfg(feature = "sound")]
    if shot.is_none() && !args.iter().any(|a| a == "--mute") {
        game.sounds = Sounds::load().await;
    }
    if shot.is_some() {
        game.panel.pin = !std::env::args().any(|a| a == "--hide-panel");
        let biome = match arg("--biome").as_deref() {
            Some("mer") => Biome::Sea,
            Some("montagne") => Biome::Mountain,
            _ => Biome::Countryside,
        };
        demo_scene(&mut game, biome);
        game.tab = match arg("--tab").as_deref() {
            Some("matter") => Tab::Matter,
            Some("cars") => Tab::Cars,
            Some("mech") => Tab::Mech,
            Some("planet") => Tab::Planet,
            _ => Tab::Blocks,
        };
        if let Some(h) = arg("--hour").and_then(|h| h.parse::<f32>().ok()) {
            game.world.set_phase(((h - 6.0).rem_euclid(24.0)) / 24.0);
        }
        if let Some(s) = arg("--season") {
            game.world.set_season(match s.as_str() {
                "ete" => Season::Summer,
                "automne" => Season::Autumn,
                "hiver" => Season::Winter,
                _ => Season::Spring,
            });
        }
        game.mode = match arg("--view").as_deref() {
            Some("thermique") => ViewMode::Thermal,
            Some("efforts") => ViewMode::Stress,
            Some("vent") => ViewMode::Wind,
            Some("pression") => ViewMode::Pressure,
            Some("hygro") => ViewMode::Humidity,
            _ => ViewMode::Normal,
        };
        if args.iter().any(|a| a == "--palette") {
            game.views = Drawer::new(true);
        }
        if args.iter().any(|a| a == "--pipes") {
            demo_pipes(&mut game);
            game.set_running(true);
            game.tab = Tab::Fluid;
        }
        if args.iter().any(|a| a == "--select") {
            game.selection = game.blocks.first().map(|b| select::Sel::Block(b.uid));
        }
        if args.iter().any(|a| a == "--bridge") {
            demo_bridge(&mut game);
        }
        if args.iter().any(|a| a == "--car") {
            game.entry.get_or_insert((360, 300));
            game.launch_car();
        }
        if args.iter().any(|a| a == "--plane") {
            game.planes.summon(&game.world);
            if let Some(p) = &mut game.planes.plane {
                p.pos = vec2(60.0, 130.0);
            }
        }
        if args.iter().any(|a| a == "--mech") {
            let was = game.unlimited_sim;
            game.unlimited_sim = true;
            demo_mech(&mut game);
            game.unlimited_sim = was;
            if args.iter().any(|a| a == "--remote") {
                game.selected_motor = game.mech.gears.iter().find(|g| g.motor.is_some()).map(|g| g.uid);
                game.views = Drawer::new(true);
            }
        }
        game.show_science = args.iter().any(|a| a == "--science");
        game.player_pin = args.iter().any(|a| a == "--player");
        game.tool = if args.iter().any(|a| a == "--loupe") { Tool::Loupe } else { Tool::Block(BlockKind::Brick) };
        // `--states N` opens the states of the palette's matter N (1 is water).
        if let Some(i) = arg("--states").and_then(|v| v.parse::<usize>().ok()).filter(|&i| i < MATTERS.len()) {
            game.tab = Tab::Matter;
            game.matter_menu = Some(i);
            game.tool = Tool::Mat(MATTERS[i].states[MATTERS[i].default]);
        }
    }
    // `--mouse x,y` pins the pointer for screenshots.
    let demo_mouse = arg("--mouse").and_then(|v| {
        let (x, y) = v.split_once(',')?;
        Some(vec2(x.parse().ok()?, y.parse().ok()?))
    });
    let mut frame = 0u32;
    let mut frame_time = 0.0f32;

    loop {
        let (sw, sh) = (screen_width(), screen_height());
        let base = View::fit(Rect::new(game.left(), TOP_H, sw - game.left(), sh - TOP_H));
        ui.begin();
        // Screenshots must not depend on where the real pointer happens to be.
        if shot.is_some() {
            ui.pressed = false;
            ui.mouse = demo_mouse.unwrap_or(vec2(-1000.0, -1000.0));
        }
        game.handle_keys();
        game.handle_camera(base, &ui);
        // `--zoom n,x,y`: n wheel notches in, on the screen point x, y.
        if shot.is_some() && frame == 0 {
            if let Some((n, at)) = arg("--zoom").and_then(|v| {
                let mut it = v.split(',').map(|t| t.parse::<f32>().ok());
                Some((it.next()??, vec2(it.next()??, it.next()??)))
            }) {
                game.camera.zoom_at(base, at, n);
            }
        }
        let view = game.camera.apply(base);
        game.handle_mouse(view, &ui);
        game.update(get_frame_time().min(0.05));
        // `--boom`: an explosion 25 cells under the water surface at x (default 360), shortly before the shot.
        if shot.is_some() && frame == 357 && args.iter().any(|a| a == "--boom") {
            let x = arg("--boom").and_then(|v| v.parse().ok()).unwrap_or(360);
            if let Some(top) = (0..H as i32).find(|&y| game.world.at(x, y) == Mat::Water) {
                game.world.explode(x, top + 25, 18);
            }
        }

        // `--fireworks`: three salvos, bursting just before the shot. `--whale`: a whale that is
        // blowing at the shot. `--firetruck x`: a fire engine coming to a fire at x.
        if shot.is_some() {
            if (320..=340).contains(&frame) && frame % 10 == 0 && args.iter().any(|a| a == "--fireworks") {
                game.fireworks.launch(vec2(260.0 + (frame - 320) as f32 * 20.0, 430.0), 3);
                game.fireworks.update(0.4);
            }
            if frame == 240 && args.iter().any(|a| a == "--whale") {
                game.whales.summon(&game.world);
            }
            if frame == 300 {
                if let Some(x) = arg("--firetruck").and_then(|v| v.parse::<f32>().ok()) {
                    game.trucks.call(&game.world, vec2(x, 300.0));
                    if let Some(t) = &mut game.trucks.truck {
                        t.x = x - t.dir * 200.0;
                        t.y = (0..H as i32).find(|&y| game.world.hard_for_car(t.x as i32, y)).unwrap_or(400) as f32;
                    }
                }
            }
        }
        // `--wreck`: the first car blows up shortly before the shot, so its pieces are in the air.
        if shot.is_some() && frame == 300 && args.iter().any(|a| a == "--wreck") {
            game.launch_car();
        }
        if shot.is_some() && frame == 345 && args.iter().any(|a| a == "--wreck") {
            if let Some(c) = game.cars.last_mut() {
                c.fate = Fate::Exploded;
            }
        }
        // `--bridge`: a truck sets off over the bridge shortly before the shot.
        if shot.is_some() && frame == 245 && args.iter().any(|a| a == "--bridge") {
            game.launch_car();
        }

        clear_background(BG);
        // Explosions shake the picture of the world (input keeps using the steady view).
        let mut shaken = view;
        if game.shake > 0.0 {
            let t = game.time;
            shaken.ox += (t * 97.0).sin() * game.shake;
            shaken.oy += (t * 83.0).cos() * game.shake;
        }
        game.draw_scene(shaken, &mut renderer, &ui);
        game.draw_ui(&mut ui, sw, sh);

        frame += 1;
        frame_time += get_frame_time();
        if let Some(path) = &shot {
            if frame == 360 {
                eprintln!("average frame time: {:.1} ms", frame_time * 1000.0 / frame as f32);
                get_screen_data().export_png(path);
                std::process::exit(0);
            }
        }
        next_frame().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat stone floor at y = 300 under a clear sky, free building, nothing else.
    fn mech_game() -> Game {
        let mut g = Game::new();
        g.world = World::new_static();
        g.empty_world();
        for x in 0..W as i32 {
            for y in 300..320 {
                g.world.set(x, y, Mat::Stone);
            }
        }
        g.unlimited = true;
        g
    }

    fn click_with(g: &mut Game, tool: Tool, x: f32, y: f32) {
        g.tool = tool;
        g.mech_click(vec2(x, y));
    }

    /// Motor + large gear meshing + a rail with a carriage joined by a rod.
    fn build_crank_slider(g: &mut Game) {
        build_crank_slider_with_pin(g, 324.0);
    }

    /// `pin_x` 324 puts the pin on the side nearest the rail (the carriage starts at the far end
    /// of its stroke), 308 on the far side (it starts at the near end and travels towards the rail).
    fn build_crank_slider_with_pin(g: &mut Game, pin_x: f32) {
        click_with(g, Tool::Motor, 300.0, 280.0);
        click_with(g, Tool::Gear(GearSize::Medium), 316.0, 280.0); // 6 + 10: meshes
        click_with(g, Tool::Rail, 330.0, 250.0);
        click_with(g, Tool::Rail, 430.0, 250.0);
        // Pin near the rim of the medium gear, then the rail.
        click_with(g, Tool::Crank, pin_x, 280.0);
        click_with(g, Tool::Crank, 380.0, 250.0);
    }

    #[test]
    fn mechanism_tools_build_a_working_crank_slider() {
        let mut g = mech_game();
        build_crank_slider(&mut g);
        assert_eq!(g.mech.gears.len(), 2, "motor + gear");
        assert!(g.mech.gears[0].motor.is_some());
        assert_eq!(g.mech.sliders.len(), 1);
        assert_eq!(g.mech.rods.len(), 1, "the rod should have been attached");
        // Placing a gear on top of another is refused.
        click_with(&mut g, Tool::Gear(GearSize::Large), 305.0, 282.0);
        assert_eq!(g.mech.gears.len(), 2);
        g.set_running(true);
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for _ in 0..900 {
            g.sim_step();
            let s = g.mech.sliders[0].s;
            lo = lo.min(s);
            hi = hi.max(s);
        }
        assert!(hi - lo > 10.0, "the carriage should shuttle: {lo}..{hi}");
        assert!(g.mech.gears[1].omega.abs() > 0.0, "the driven gear turns");
        let ratio = g.mech.gears[1].omega / g.mech.gears[0].omega;
        assert!((ratio + 0.6).abs() < 1e-3, "6/10 reduction, opposite sense: {ratio}");
    }

    #[test]
    fn a_steam_motor_turns_on_the_pressure_of_its_boiler_and_uses_up_the_steam() {
        let run = |steam: bool| {
            let mut g = mech_game();
            // A closed stone boiler, its top half full of steam (or of plain air).
            for x in 200..=260 {
                for y in 200..=260 {
                    let wall = x == 200 || x == 260 || y == 200 || y == 260;
                    let m = if wall { Mat::Stone } else if steam && y < 215 { Mat::Steam } else { Mat::Empty };
                    g.world.set(x, y, m);
                }
            }
            click_with(&mut g, Tool::Motor, 230.0, 240.0);
            g.mech.gears[0].motor.as_mut().unwrap().steam = true;
            let count = |g: &Game| g.world.census()[Mat::Steam as usize];
            let before = count(&g);
            g.set_running(true);
            let mut spin = 0.0f32;
            for _ in 0..400 {
                g.sim_step();
                spin = spin.max(g.mech.gears[0].omega.abs());
            }
            (spin, before, count(&g), g.mech.gears[0].motor.unwrap().pressure)
        };
        let (spin, before, after, atm) = run(true);
        assert!(atm > 1.5 && spin > 0.03, "the boiler drives the motor: {spin} rad/step at {atm} atm");
        assert!(after < before, "the motor uses up the steam: {before} -> {after}");
        let (idle, ..) = run(false);
        assert!(idle < 0.005, "no steam, no torque: {idle}");
    }

    #[test]
    fn mechanisms_undo_reset_and_erase() {
        let mut g = mech_game();
        click_with(&mut g, Tool::Gear(GearSize::Small), 200.0, 280.0);
        click_with(&mut g, Tool::Gear(GearSize::Small), 400.0, 280.0);
        assert_eq!(g.mech.gears.len(), 2);
        g.undo();
        assert_eq!(g.mech.gears.len(), 1, "undo removes the last piece");
        assert!(g.mech_erase(vec2(200.0, 280.0)));
        assert!(g.mech.gears.is_empty());
        // Running changes angles; going back restores the drawing board.
        click_with(&mut g, Tool::Motor, 300.0, 280.0);
        g.set_running(true);
        for _ in 0..100 {
            g.sim_step();
        }
        assert!(g.mech.gears[0].angle.abs() > 0.1);
        g.reset();
        assert_eq!(g.mech.gears[0].angle, 0.0);
    }

    #[test]
    fn a_carriage_shoves_sand_out_of_its_way() {
        let mut g = mech_game();
        // The carriage shuttles about x 372..388 on a rail along y = 250, with sand in its way
        // and a shelf under the sand so that it stays at the carriage's height.
        for y in 244..258 {
            for x in 387..392 {
                g.world.set(x, y, Mat::Sand);
            }
        }
        for x in 355..420 {
            for y in 258..262 {
                g.world.set(x, y, Mat::Stone);
            }
        }
        build_crank_slider_with_pin(&mut g, 308.0);
        g.mech.rods[0].strength = 1e6; // this test is about the sand, not the rod
        g.set_running(true);
        let sand_before = g.world.census()[Mat::Sand as usize];
        for _ in 0..900 {
            g.sim_step();
        }
        assert!(g.world.census()[Mat::Sand as usize] >= sand_before - 2, "the sand is pushed, not destroyed");
        let moved = (244..258).flat_map(|y| (360..420).map(move |x| (x, y))).filter(|&(x, y)| g.world.raw(x, y) == Mat::Sand && !(387..392).contains(&x)).count();
        assert!(moved > 0, "some sand should have been shoved aside");
    }

    #[test]
    fn a_carriage_cannot_go_through_rock() {
        let mut g = mech_game();
        // The carriage would travel from x 380 up to about 396; the rock wall is in the way.
        for y in 220..270 {
            g.world.set(392, y, Mat::Stone);
        }
        build_crank_slider_with_pin(&mut g, 308.0);
        g.set_running(true);
        let mut right_edge = f32::MIN;
        for _ in 0..1500 {
            g.sim_step();
            let sl = &g.mech.sliders[0];
            right_edge = right_edge.max(sl.point().x + sl.half.x);
        }
        assert!(right_edge < 392.5, "the carriage must stop at the rock: right edge {right_edge}");
    }

    #[test]
    fn the_editor_helps_to_stick_mechanism_pieces_in_the_right_place() {
        let mut g = mech_game();
        click_with(&mut g, Tool::Gear(GearSize::Medium), 300.0, 280.0);
        // A small gear dropped roughly beside it clips to the exact centre distance (10 + 6)...
        let c = g.gear_spot(GearSize::Small, vec2(318.5, 281.0));
        assert!(((c - vec2(300.0, 280.0)).length() - 16.0).abs() < 1e-3, "mesh distance: {c:?}");
        // ...and a nearly horizontal approach is straightened out.
        assert!((c.y - 280.0).abs() < 1e-3, "straightened: {c:?}");
        // Far from any gear, it only lines up with another gear's centre line.
        let c = g.gear_spot(GearSize::Small, vec2(400.0, 281.5));
        assert_eq!(c, vec2(400.0, 280.0));
        // Rails straighten out in the air.
        let (a, p) = g.rail_end(vec2(100.0, 100.0), vec2(187.0, 103.0));
        assert!(matches!(a, Anchor::Free(_)));
        assert!((p.y - 100.0).abs() < 1e-3 && (p.x - 185.0).abs() < 1e-3, "{p:?}");
        // The pin of a crank jumps to the nearest attach point of the gear.
        let gi = 0;
        let spot = g.pin_spot_for_test(gi, vec2(300.0 + 8.2, 280.0 + 0.7));
        assert!(((spot - vec2(308.0, 280.0)).length()) < 0.6, "attach point: {spot:?}");
    }

    #[test]
    fn a_motor_a_shaft_and_a_pivoted_wheel_make_a_winch() {
        let mut g = mech_game();
        // A drum high up on a pivot in the air, a cube hanging from a rope fixed on its rim.
        g.blocks.push(Block::new(BlockKind::Wheel, vec2(300.0, 150.0), 0.0));
        g.blocks.push(Block::new(BlockKind::Cube, vec2(288.0, 260.0), 0.0));
        g.refresh_mask();
        click_with(&mut g, Tool::Pivot, 300.0, 150.0);
        assert_eq!(g.mech.pivots.len(), 1);
        click_with(&mut g, Tool::Motor, 360.0, 150.0);
        click_with(&mut g, Tool::Axle, 360.0, 150.0);
        click_with(&mut g, Tool::Axle, 300.0, 150.0);
        assert!(g.mech.gears[0].wheel.is_some(), "the shaft links the motor to the drum");
        g.cable_click(CableMat::Iron, vec2(289.0, 151.0));
        g.cable_click(CableMat::Iron, vec2(288.0, 252.0));
        assert_eq!(g.cables.len(), 1);
        match g.cables[0].a {
            Anchor::Block { local, .. } => assert!((local.length() - 12.0).abs() < 1e-3, "rope fixed on the rim: {local:?}"),
            _ => panic!("rope not fixed to the drum"),
        }
        g.set_running(true);
        let start = g.blocks[1].pos.y;
        for _ in 0..1800 {
            g.sim_step();
        }
        eprintln!("WINCH2 cube {start:.1} -> {:.1}, wheel {:?}, snapped {}", g.blocks[1].pos.y, g.blocks[0].pos, g.cables[0].snapped);
        assert!((g.blocks[0].pos - vec2(300.0, 150.0)).length() < 2.0, "drum stays on its axle");
        assert!(g.blocks[1].pos.y < start - 30.0, "the winch lifts the cube: {start} -> {}", g.blocks[1].pos.y);
    }

    #[test]
    fn a_cable_can_be_left_loose_at_one_end_and_survives_a_save() {
        let mut g = mech_game();
        g.cable_click(CableMat::Wood, vec2(200.0, 300.0)); // fixed on the ground
        g.cable_click(CableMat::Wood, vec2(260.0, 240.0)); // nothing there: loose end
        assert_eq!(g.cables.len(), 1);
        assert!(matches!(g.cables[0].b, Anchor::Free(_)));
        // A first click in the air is still refused.
        g.cable_click(CableMat::Wood, vec2(100.0, 100.0));
        assert!(g.cable_from.is_none());
        assert_eq!(g.cables.len(), 1);
        let bytes = save::encode(&g.world, None, None, &g.blocks, &g.cables, &g.members, &g.springs, &g.mech);
        let l = save::decode(&bytes).unwrap();
        assert_eq!(l.cables.len(), 1, "the half-loose cable comes back");
        assert!(matches!(l.cables[0].b, Anchor::Free(_)));
    }

    /// How far a tige driven by a gear lags behind it (radians), given what its pivot is fixed to.
    fn arm_lag(g: &Game, gi: usize) -> f32 {
        let gear = &g.mech.gears[gi];
        let (uid, end) = gear.arm.unwrap();
        let m = g.members.iter().find(|m| m.uid == uid).unwrap();
        let support = match if end == 0 { m.a } else { m.b } {
            Anchor::End { uid, .. } | Anchor::Along { uid, .. } => g.members.iter().find(|o| o.uid == uid).unwrap().angle(),
            _ => 0.0,
        };
        let (g0, a0) = gear.arm_base.unwrap();
        let d = m.angle() - support - (a0 + gear.angle - g0);
        (d + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
    }

    #[test]
    fn a_pivot_on_the_end_of_a_tige_and_a_shaft_from_a_motor_make_a_swinging_arm() {
        let mut g = mech_game();
        let (p0, p1) = (vec2(300.0, 200.0), vec2(370.0, 200.0));
        g.members.push(Member::new(MemberMat::Steel, Anchor::Free(p0), Anchor::Free(p1), p0, p1));
        click_with(&mut g, Tool::Pivot, 301.0, 200.0);
        assert!(matches!(g.members[0].a, Anchor::Ground(_)), "the loose end gets an axle in the air");
        assert!(matches!(g.members[0].b, Anchor::Free(_)));
        // A large gear on the pivot, geared down from a motor just above it.
        click_with(&mut g, Tool::Gear(GearSize::Large), 300.0, 200.0);
        click_with(&mut g, Tool::Motor, 300.0, 180.0);
        // Axle tool: the gear, then the gear again (the tige end it sits on).
        click_with(&mut g, Tool::Axle, 300.0, 200.0);
        click_with(&mut g, Tool::Axle, 300.0, 200.0);
        assert_eq!(g.mech.gears[0].arm, Some((g.members[0].uid, 0)), "the gear swings the tige about its fixed end");
        // It comes back from a save.
        let bytes = save::encode(&g.world, None, None, &g.blocks, &g.cables, &g.members, &g.springs, &g.mech);
        let l = save::decode(&bytes).unwrap();
        assert_eq!(l.mech.gears[0].arm, Some((l.members[0].uid, 0)));
        assert!(matches!(l.members[0].a, Anchor::Ground(_)));
        g.set_running(true);
        let mut worst = 0.0f32;
        for t in 0..1500 {
            g.sim_step();
            if t > 30 {
                worst = worst.max(arm_lag(&g, 0).abs());
            }
        }
        eprintln!("ARM gear {:.2} lag {worst:.3} pivot {:?} tip {:?}", g.mech.gears[0].angle, g.members[0].p0, g.members[0].p1);
        assert!(g.mech.gears[0].angle.abs() > 4.0, "the gear (and the arm) went round: {}", g.mech.gears[0].angle);
        assert!(worst < 0.15, "the arm follows its gear: lag {worst}");
        assert!((g.members[0].p0 - p0).length() < 2.0, "the pivot stays put: {:?}", g.members[0].p0);
        // Pivot tool again on the axle in the air: it comes off, and the shaft with it.
        g.set_running(false);
        let pivot = g.members[0].p0;
        click_with(&mut g, Tool::Pivot, pivot.x, pivot.y);
        assert!(matches!(g.members[0].a, Anchor::Free(_)));
        assert_eq!(g.mech.gears[0].arm, None);
    }

    #[test]
    fn a_tige_on_the_end_of_an_arm_makes_an_articulated_arm() {
        let mut g = mech_game();
        // Shoulder on the ground, elbow at the end of the first tige, forearm hanging from it.
        let (s, e, h) = (vec2(300.0, 299.0), vec2(300.0, 230.0), vec2(360.0, 230.0));
        g.members.push(Member::new(MemberMat::Steel, Anchor::Ground(s), Anchor::Free(e), s, e));
        let upper = g.members[0].uid;
        g.members.push(Member::new(MemberMat::Wood, Anchor::End { uid: upper, end: 1 }, Anchor::Free(h), e, h));
        // Each joint has its own motor, through a large gear, somewhere off to the side.
        for (y, along) in [(150.0, vec2(300.0, 260.0)), (100.0, vec2(330.0, 230.0))] {
            click_with(&mut g, Tool::Gear(GearSize::Large), 200.0, y);
            click_with(&mut g, Tool::Motor, 180.0, y);
            click_with(&mut g, Tool::Axle, 200.0, y);
            click_with(&mut g, Tool::Axle, along.x, along.y); // the middle of a tige: its fixed end is the joint
        }
        let large: Vec<usize> = (0..g.mech.gears.len()).filter(|&i| g.mech.gears[i].size == GearSize::Large).collect();
        assert_eq!(g.mech.gears[large[0]].arm, Some((upper, 0)));
        assert_eq!(g.mech.gears[large[1]].arm, Some((g.members[1].uid, 0)), "the forearm turns about the elbow");
        g.mech.gears.iter_mut().filter(|x| x.motor.is_some()).for_each(|x| x.motor.as_mut().unwrap().free_speed = 0.05);
        g.set_running(true);
        let mut worst = [0.0f32; 2];
        let start = g.members[1].p1;
        let mut reach = 0.0f32;
        for t in 0..1200 {
            g.sim_step();
            if t > 30 {
                for k in 0..2 {
                    worst[k] = worst[k].max(arm_lag(&g, large[k]).abs());
                }
            }
            reach = reach.max((g.members[1].p1 - start).length());
        }
        eprintln!("ELBOW lags {worst:?} reach {reach:.1} elbow {:?}", g.members[0].p1);
        assert!(worst[0] < 0.2 && worst[1] < 0.2, "both joints follow their gears: {worst:?}");
        assert!(reach > 40.0, "the hand travels: {reach}");
        assert!((g.members[0].p1 - g.members[1].p0).length() < 2.0, "the elbow holds together");
    }

    #[test]
    fn the_screw_tool_turns_a_rail_into_a_lead_screw_that_survives_a_save() {
        let mut g = mech_game();
        click_with(&mut g, Tool::Gear(GearSize::Large), 300.0, 250.0);
        click_with(&mut g, Tool::Motor, 300.0, 230.0);
        click_with(&mut g, Tool::Rail, 330.0, 250.0);
        click_with(&mut g, Tool::Rail, 430.0, 250.0);
        // The rail first, then the gear.
        click_with(&mut g, Tool::Screw, 380.0, 250.0);
        click_with(&mut g, Tool::Screw, 300.0, 250.0);
        let gear = g.mech.gears[0].uid;
        assert_eq!(g.mech.sliders[0].screw, Some(gear));
        let bytes = save::encode(&g.world, None, None, &g.blocks, &g.cables, &g.members, &g.springs, &g.mech);
        assert_eq!(save::decode(&bytes).unwrap().mech.sliders[0].screw, Some(gear));
        g.set_running(true);
        let (s0, a0) = (g.mech.sliders[0].s, g.mech.gears[0].angle);
        for _ in 0..600 {
            g.sim_step();
        }
        let moved = g.mech.sliders[0].s - s0;
        let want = Mechanism::screw_travel(g.mech.gears[0].angle - a0);
        eprintln!("SCREW moved {moved:.2} want {want:.2}");
        assert!(moved.abs() > 5.0, "the nut travels: {moved}");
        assert!((moved - want).abs() < 0.05, "one lead per turn: {moved} vs {want}");
        // The eraser on the shaft between gear and screw makes it a plain rail again.
        g.set_running(false);
        assert!(g.mech_erase(vec2(318.0, 250.0)));
        assert_eq!(g.mech.sliders[0].screw, None);
        assert_eq!(g.mech.gears.len(), 2, "the gears are still there");
    }

    #[test]
    fn a_connecting_rod_can_be_pinned_to_a_block_and_pushes_it() {
        let mut g = mech_game();
        g.blocks.push(Block::new(BlockKind::Cube, vec2(380.0, 290.0), 0.0));
        g.refresh_mask();
        click_with(&mut g, Tool::Motor, 300.0, 270.0);
        click_with(&mut g, Tool::Gear(GearSize::Medium), 316.0, 270.0);
        click_with(&mut g, Tool::Crank, 322.0, 266.0); // an attach point on the medium gear
        click_with(&mut g, Tool::Crank, 372.0, 290.0); // the cube
        assert_eq!(g.mech.rods.len(), 1, "a rod to a block is allowed");
        assert!(matches!(g.mech.rods[0].b, mech::End::Node(Anchor::Block { .. })));
        g.mech.rods[0].strength = 1e6;
        g.set_running(true);
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for t in 0..900 {
            g.sim_step();
            if t > 120 {
                lo = lo.min(g.blocks[0].pos.x);
                hi = hi.max(g.blocks[0].pos.x);
            }
        }
        eprintln!("BLOCKROD cube x {lo:.1}..{hi:.1}, y {:.1}, gear turned {:.1}, broken {}", g.blocks[0].pos.y, g.mech.gears[1].angle, g.mech.rods[0].broken);
        assert!(hi - lo > 3.0, "the rod drags the block to and fro: {lo}..{hi}");
        assert!(g.mech.gears[1].angle.abs() > 1.0, "and the gear still turns");
    }

    #[test]
    fn a_winch_reels_a_cable_in_and_stops_when_there_is_no_rope_left() {
        let mut g = mech_game();
        g.blocks.push(Block::new(BlockKind::Cube, vec2(316.0, 230.0), 0.0));
        g.refresh_mask();
        // A winch drum high up, driven by a motor beside it (meshing reverses, so turn the motor the other way).
        click_with(&mut g, Tool::Winch, 316.0, 120.0);
        click_with(&mut g, Tool::Motor, 300.0, 120.0);
        g.mech.gears[1].motor.as_mut().unwrap().dir = -1.0;
        assert!(g.mech.gears[0].reel.is_some() && g.mech.gears[1].motor.is_some());
        g.cable_click(CableMat::Iron, vec2(316.0, 121.0)); // the drum
        g.cable_click(CableMat::Iron, vec2(316.0, 222.0)); // the cube
        assert_eq!(g.cables.len(), 1);
        assert!(matches!(g.cables[0].a, Anchor::Reel { .. }), "the rope is fixed on the drum");
        g.set_running(true);
        let start = g.blocks[0].pos.y;
        let start_len = g.cables[0].len;
        let mut stalled = false;
        for _ in 0..2500 {
            g.sim_step();
            stalled |= g.mech.gears[0].omega.abs() < 1e-6 && g.cables[0].len < 20.0;
        }
        eprintln!("WINCH3 cube {start:.0} -> {:.0}, rope {start_len:.0} -> {:.0}, drum angle {:.1}", g.blocks[0].pos.y, g.cables[0].len, g.mech.gears[0].angle);
        assert!(g.blocks[0].pos.y < start - 60.0, "the winch hauls the cube up");
        assert!(g.cables[0].len >= 13.9, "no more rope than there is: {}", g.cables[0].len);
        assert!(g.cables[0].len < start_len - 60.0, "the rope was reeled in");
        assert!(stalled, "the drum stops (the motor stalls) when the rope is all in");
    }

    #[test]
    fn a_cable_runs_over_a_pulley_and_carries_weight_both_sides() {
        let mut g = mech_game();
        g.blocks.push(Block::new(BlockKind::Cube, vec2(280.0, 230.0), 0.0));
        g.blocks.push(Block::new(BlockKind::Glass, vec2(320.0, 230.0), 0.0));
        g.refresh_mask();
        click_with(&mut g, Tool::Pulley, 300.0, 110.0);
        assert_eq!(g.mech.pulleys.len(), 1);
        g.cable_click(CableMat::Iron, vec2(280.0, 222.0)); // first block
        g.cable_click(CableMat::Iron, vec2(300.0, 110.0)); // the pulley
        assert_eq!(g.cable_via.len(), 1);
        g.cable_click(CableMat::Iron, vec2(320.0, 226.0)); // second block
        assert_eq!(g.cables.len(), 1);
        assert_eq!(g.cables[0].via.len(), 1);
        assert_eq!(g.cables[0].via_pts.len(), 1, "where the rope touches the pulley");
        assert!(g.cables[0].via_pts[0].y < 110.0, "over the top of the pulley");
        g.set_running(true);
        let (y0, y1) = (g.blocks[0].pos.y, g.blocks[1].pos.y);
        let (mut down, mut up) = (0.0f32, 0.0f32);
        for _ in 0..100 {
            g.sim_step();
            down = down.max(g.blocks[0].pos.y - y0);
            up = up.max(y1 - g.blocks[1].pos.y);
        }
        assert!(down > 15.0 && up > 10.0, "the heavy side falls, the light side rises: {down} {up}");
        // The pulley and its rope survive a save.
        let bytes = save::encode(&g.world, None, None, &g.blocks, &g.cables, &g.members, &g.springs, &g.mech);
        let l = save::decode(&bytes).unwrap();
        assert_eq!(l.mech.pulleys.len(), 1);
        assert_eq!(l.cables[0].via, vec![l.mech.pulleys[0].uid]);
    }

    /// Two stone vessels on the floor, the left one 90 deep in water, the right one 20; a pipework from
    /// one to the other drawn with the pipe tool, over a hump 60 cells above the left surface.
    fn plumbed_vessels() -> Game {
        let mut g = mech_game();
        for (x0, x1, top) in [(100, 160, 210), (300, 360, 280)] {
            for y in 180..300 {
                for x in x0 - 8..x0 {
                    g.world.set(x, y, Mat::Stone);
                }
                for x in x1..x1 + 8 {
                    g.world.set(x, y, Mat::Stone);
                }
            }
            for y in top..300 {
                for x in x0..x1 {
                    g.world.set(x, y, Mat::Water);
                }
            }
        }
        g.tool = Tool::Pipe;
        for (x, y) in [(140.0, 290.0), (140.0, 150.0), (320.0, 150.0), (320.0, 290.0)] {
            g.fluid_click(vec2(x, y));
        }
        g.pipe_from = None;
        g
    }

    fn level(g: &Game, x: i32) -> f32 {
        let y = (0..H as i32).find(|&y| g.world.raw(x, y) == Mat::Water).unwrap_or(H as i32);
        H as f32 - y as f32
    }

    #[test]
    fn pipes_drawn_with_the_tool_level_two_vessels_through_a_siphon() {
        let mut g = plumbed_vessels();
        assert_eq!(g.mech.plumbing.pipes.len(), 3, "three straight pipes");
        let before = (level(&g, 130), level(&g, 330));
        g.set_running(true);
        for _ in 0..4000 {
            g.sim_step();
        }
        let (l, r) = (level(&g, 130), level(&g, 330));
        eprintln!("PIPES levels {before:?} -> {l} / {r}");
        assert!(before.0 - before.1 > 50.0);
        assert!((l - r).abs() < 6.0, "the vessels reach the same level: {l} vs {r}");
        // Going back to the drawing board puts the water where it was.
        g.reset();
        assert!((level(&g, 130) - before.0).abs() < 1.0);
    }

    fn drag_from_to(g: &mut Game, from: Vec2, to: Vec2) {
        g.tool = Tool::Select;
        g.select_mouse(from, true, true, false);
        let steps = 6;
        for k in 1..=steps {
            g.select_mouse(from + (to - from) * (k as f32 / steps as f32), false, true, false);
        }
        g.drag = None;
    }

    #[test]
    fn the_selection_tool_picks_moves_resizes_and_deletes() {
        let mut g = mech_game();
        // A block: picked, moved (clipping onto the floor), deleted.
        g.blocks.push(Block::new(BlockKind::Cube, vec2(200.0, 290.0), 0.0));
        g.refresh_mask();
        g.tool = Tool::Select;
        g.select_mouse(vec2(200.0, 290.0), true, false, false);
        assert!(matches!(g.selection, Some(select::Sel::Block(_))));
        g.drag = None;
        drag_from_to(&mut g, vec2(200.0, 290.0), vec2(300.0, 290.0));
        assert!((g.blocks[0].pos.x - 300.0).abs() < 6.0, "the block followed the drag: {:?}", g.blocks[0].pos);
        assert!(g.blocks[0].pos.y > 285.0, "and stayed on the floor: {:?}", g.blocks[0].pos);
        // A pipe: its end is dragged out to make it longer.
        g.tool = Tool::Pipe;
        g.fluid_click(vec2(100.0, 200.0));
        g.fluid_click(vec2(140.0, 200.0));
        g.pipe_from = None;
        let len_before = g.mech.plumbing.pipes[0].len();
        drag_from_to(&mut g, vec2(141.0, 200.0), vec2(200.0, 200.0));
        let len_after = g.mech.plumbing.pipes[0].len();
        assert!(len_after > len_before + 30.0, "the pipe was stretched by its handle: {len_before} -> {len_after}");
        // Moving the whole pipe by its body.
        let a0 = g.mech.plumbing.pipes[0].a;
        drag_from_to(&mut g, vec2(150.0, 200.0), vec2(150.0, 240.0));
        assert!((g.mech.plumbing.pipes[0].a.1 - a0.1 - 40.0).abs() < 1.5, "the pipe moved down by 40");
        // A gear moves, and a rod attached to it stays attached (its length is refitted).
        click_with(&mut g, Tool::Gear(GearSize::Medium), 400.0, 250.0);
        click_with(&mut g, Tool::Rail, 430.0, 200.0);
        click_with(&mut g, Tool::Rail, 530.0, 200.0);
        click_with(&mut g, Tool::Crank, 408.0, 250.0);
        click_with(&mut g, Tool::Crank, 480.0, 200.0);
        assert_eq!(g.mech.rods.len(), 1);
        drag_from_to(&mut g, vec2(400.0, 250.0), vec2(400.0, 270.0));
        let r = &g.mech.rods[0];
        let (pa, pb) = (g.rod_end_world(&r.a).unwrap(), g.rod_end_world(&r.b).unwrap());
        assert!(((pa - pb).length() - r.len).abs() < 0.5, "the rod still fits its two ends");
        assert!((g.mech.gears[0].pos.y - 270.0).abs() < 2.5, "the gear moved: {:?}", g.mech.gears[0].pos);
        // Delete, and undo brings it back.
        g.selection = Some(select::Sel::Gear(g.mech.gears[0].uid));
        g.select_delete();
        assert!(g.mech.gears.is_empty() && g.mech.rods.is_empty(), "the gear and its rod are gone");
        g.undo();
        assert_eq!(g.mech.gears.len(), 1);
        // Nothing is edited while the simulation runs.
        g.set_running(true);
        let pos = g.mech.gears[0].pos;
        g.tool = Tool::Select;
        g.select_mouse(pos, true, true, false);
        g.select_mouse(pos + vec2(30.0, 0.0), false, true, false);
        assert_eq!(g.mech.gears[0].pos, pos);
    }

    #[test]
    fn a_pump_is_set_on_a_pipe_turned_round_stopped_and_driven_by_a_gear_and_survives_a_save() {
        let mut g = plumbed_vessels();
        g.tool = Tool::Pump;
        g.fluid_click(vec2(230.0, 150.0));
        assert_eq!(g.mech.plumbing.pumps.len(), 1);
        assert!(g.mech.plumbing.pumps[0].on && g.mech.plumbing.pumps[0].head > 0.0);
        g.fluid_click(vec2(230.0, 150.0));
        assert!(g.mech.plumbing.pumps[0].head < 0.0, "turned round");
        g.fluid_click(vec2(230.0, 150.0));
        assert!(!g.mech.plumbing.pumps[0].on, "stopped");
        g.fluid_click(vec2(230.0, 150.0));
        assert!(g.mech.plumbing.pumps[0].on, "running again");
        // A motor drives it through the axle tool.
        click_with(&mut g, Tool::Motor, 230.0, 100.0);
        click_with(&mut g, Tool::Axle, 230.0, 100.0);
        click_with(&mut g, Tool::Axle, 230.0, 150.0);
        let gear = g.mech.gears[0].uid;
        assert_eq!(g.mech.plumbing.pumps[0].driver, Some(gear));
        g.set_running(true);
        // (The motor turns back now and then under the load, so look at the strongest lift.)
        let mut lift = 0.0f32;
        for _ in 0..200 {
            g.sim_step();
            lift = lift.max(g.mech.plumbing.pumps[0].head.abs());
        }
        assert!(lift > 20.0, "the lift follows the gear's speed: {lift}");
        let bytes = save::encode(&g.world, None, None, &g.blocks, &g.cables, &g.members, &g.springs, &g.mech);
        let l = save::decode(&bytes).unwrap();
        assert_eq!(l.mech.plumbing.pumps.len(), 1);
        assert_eq!(l.mech.plumbing.pumps[0].driver, Some(gear));
    }

    #[test]
    fn a_valve_turned_by_a_motor_closes_after_its_turns_and_stalls_the_motor() {
        let mut g = plumbed_vessels();
        // A valve in the middle of the horizontal pipe, a motor above it, joined with the axle tool.
        g.tool = Tool::Valve;
        g.fluid_click(vec2(230.0, 150.0));
        assert_eq!(g.mech.plumbing.valves.len(), 1);
        click_with(&mut g, Tool::Motor, 230.0, 100.0);
        g.mech.gears[0].motor.as_mut().unwrap().dir = -1.0; // closing
        click_with(&mut g, Tool::Axle, 230.0, 100.0);
        click_with(&mut g, Tool::Axle, 230.0, 150.0);
        assert!(matches!(g.mech.plumbing.valves[0].driver, Some(pipes::Driver::Gear(_))), "the valve is turned by the gear");
        g.mech.plumbing.valves[0].turns = 2.0;
        g.set_running(true);
        let mut min_open: f32 = 1.0;
        let mut stalled = false;
        for _ in 0..3000 {
            g.sim_step();
            min_open = min_open.min(g.mech.plumbing.valves[0].open);
            stalled |= g.mech.gears[0].omega.abs() < 1e-6 && g.mech.plumbing.valves[0].open < 0.01;
        }
        assert!(min_open < 0.01, "the valve ends up shut: {min_open}");
        assert!(stalled, "and the motor stalls against the end stop");
        // Shut for good: the high vessel keeps most of its water.
        assert!(level(&g, 130) > 300.0, "water stayed in the left vessel: {}", level(&g, 130));
    }

    #[test]
    fn the_post_is_a_tall_thin_brick_standing_up() {
        let mut g = mech_game();
        assert!(BlockKind::PLACEABLE.contains(&BlockKind::Post), "it is in the palette");
        let spec = BlockKind::Post.spec();
        let long = BlockKind::Long.spec();
        assert_eq!((spec.w, spec.h), (long.h, long.w), "the long brick, but vertical");
        g.blocks.push(Block::new(BlockKind::Post, vec2(300.0, 270.0), 0.0));
        assert_eq!(g.blocks.len(), 1);
        assert_eq!(g.blocks[0].kind, BlockKind::Post);
        assert_eq!(g.blocks[0].angle, 0.0, "no rotation is needed to stand it up");
        let bytes = save::encode(&g.world, None, None, &g.blocks, &g.cables, &g.members, &g.springs, &g.mech);
        let l = save::decode(&bytes).unwrap();
        assert_eq!(l.blocks[0].kind, BlockKind::Post);
        assert_eq!(BlockKind::from_id(BlockKind::Post.id()), Some(BlockKind::Post));
    }

    #[test]
    fn the_shovel_scoops_a_heap_and_pours_it_elsewhere_with_its_heat() {
        let mut g = mech_game();
        // A hot heap of ash on the floor.
        for x in 200..212 {
            for y in 292..300 {
                g.world.set(x, y, Mat::Ash);
                g.world.temp[y as usize * W + x as usize] = 480.0;
            }
        }
        let ash = |g: &Game, x0: i32, x1: i32| (0..H as i32).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| g.world.raw(x, y) == Mat::Ash).count();
        assert_eq!(ash(&g, 190, 230), 96);
        g.brush = 6;
        g.tool = Tool::Shovel;
        g.shovel_scoop(vec2(205.0, 296.0));
        let load = g.shovel.expect("something was scooped");
        assert_eq!(load.mat, Mat::Ash);
        assert!(load.count >= 30 && load.count <= 96, "a scoopful: {}", load.count);
        assert!(load.temp > 400.0, "the ash stays hot on the shovel: {}", load.temp);
        assert_eq!(ash(&g, 190, 230), 96 - load.count as usize, "what was taken is gone from the heap");
        // A full shovel does not fill again: a click only lets go of a part of its load.
        let before = g.shovel.unwrap().count;
        // A quick click lets go of a pinch; held longer, of a bigger part.
        let quick = {
            g.shovel_mouse(vec2(205.0, 296.0), true, false);
            g.shovel_release(vec2(205.0, 296.0));
            before - g.shovel.map_or(0, |l| l.count)
        };
        assert!(quick > 0 && quick <= 8, "a quick click pours a pinch: {quick}");
        let mid = g.shovel.unwrap();
        assert!(Game::shovel_portion(&mid, 0.7) > Game::shovel_portion(&mid, 0.1), "the longer it is held the bigger the part");
        assert_eq!(Game::shovel_portion(&mid, 5.0), mid.count, "held long enough it is everything");
        g.shovel_mouse(vec2(205.0, 296.0), true, false);
        g.shovel_charge = Some(0.75);
        g.shovel_release(vec2(205.0, 296.0));
        let after = g.shovel.map_or(0, |l| l.count);
        assert!(after < mid.count - quick, "a held click pours a bigger part: {} -> {after}", mid.count);
        assert_eq!(ash(&g, 0, W as i32) + after as usize, 96, "what poured went back into the world; nothing was scooped again");
        // Poured out elsewhere, all of it, nothing lost.
        g.shovel_dump(vec2(400.0, 295.0));
        assert!(g.shovel.is_none());
        assert_eq!(ash(&g, 380, 420), after as usize);
        assert_eq!(ash(&g, 0, W as i32), 96, "the matter is moved, not created or lost");
        let hot = (380..420).flat_map(|x| (0..H as i32).map(move |y| (x, y))).filter(|&(x, y)| g.world.raw(x, y) == Mat::Ash && g.world.temp[y as usize * W + x as usize] > 400.0).count();
        assert!(hot > 0, "the poured ash is still hot");
        // A shovel with nothing in reach says so.
        g.shovel_scoop(vec2(600.0, 100.0));
        assert!(g.shovel.is_none());
    }

    #[test]
    fn slow_frames_simplify_the_physics_before_slowing_the_game_and_smooth_ones_restore_it() {
        let mut g = mech_game();
        g.set_running(true);
        g.speed = 3;
        let mut said = vec![];
        for _ in 0..400 {
            g.update(0.05);
            if let Some((m, t)) = &g.toast {
                if *t > 1.5 && said.last() != Some(m) {
                    said.push(m.clone());
                }
            }
        }
        assert_eq!(g.world.lod, 3, "all the simplifications come first");
        assert!(g.speed < 3, "then the speed drops");
        assert!(said.iter().any(|m| m.contains("Simplification 1/3")) && said.iter().any(|m| m.contains("3/3")), "each one is announced: {said:?}");
        // A long smooth stretch brings the exact simulation back, a level at a time.
        g.lod_lock = 0.0;
        for _ in 0..4000 {
            g.update(0.004);
            g.lod_lock = 0.0;
        }
        assert_eq!(g.world.lod, 0, "the exact physics is back");
    }

    #[test]
    fn escape_backs_out_one_level_at_a_time_and_pauses_before_leaving() {
        let mut g = mech_game();
        g.set_running(true);
        // A build in progress is cancelled first.
        g.cable_from = Some(Anchor::Free(vec2(10.0, 10.0)));
        g.on_escape();
        assert!(g.cable_from.is_none() && !g.quit_modal && g.running);
        // Then the full screen (not toggled for real in a test).
        g.fullscreen = false;
        // Then the window that asks to leave, with the simulation paused.
        g.on_escape();
        assert!(g.quit_modal && !g.running, "the leave window opens and pauses the game");
        // Esc again goes back and the simulation runs again.
        g.on_escape();
        assert!(!g.quit_modal && g.running, "the game resumes where it was");
        // A game that was paused stays paused after the window closes.
        g.set_running(false);
        g.on_escape();
        g.on_escape();
        assert!(!g.quit_modal && !g.running);
        // Enter validates the selected button: "Reprendre" (the default) goes back, "Quitter" leaves.
        g.on_escape();
        assert!(!g.on_enter_in_quit_modal() && !g.quit_modal, "Enter on the default button goes back");
        g.on_escape();
        g.quit_choice = true;
        assert!(g.on_enter_in_quit_modal() && g.quit_modal, "Enter on Quitter leaves");
        g.on_escape();
        assert!(!g.on_enter_in_quit_modal(), "Enter does nothing outside the window");
    }

    #[test]
    fn pivots_and_wheel_shafts_survive_a_save() {
        let mut g = mech_game();
        g.blocks.push(Block::new(BlockKind::Wheel, vec2(300.0, 150.0), 0.0));
        g.refresh_mask();
        click_with(&mut g, Tool::Pivot, 300.0, 150.0);
        click_with(&mut g, Tool::Motor, 360.0, 150.0);
        click_with(&mut g, Tool::Axle, 360.0, 150.0);
        click_with(&mut g, Tool::Axle, 300.0, 150.0);
        let bytes = save::encode(&g.world, None, None, &g.blocks, &g.cables, &g.members, &g.springs, &g.mech);
        let l = save::decode(&bytes).unwrap();
        assert_eq!(l.mech.pivots.len(), 1);
        assert_eq!(l.mech.pivots[0].wheel, l.blocks[0].uid);
        assert_eq!(l.mech.gears[0].wheel, Some(l.blocks[0].uid));
        // Taking the wheel away takes the axle and the shaft with it.
        g.remove_block(0);
        g.mech_prune_wheels();
        assert!(g.mech.pivots.is_empty() && g.mech.gears[0].wheel.is_none());
    }

    #[test]
    fn mechanisms_survive_save_and_load() {
        let mut g = mech_game();
        build_crank_slider(&mut g);
        g.axle_for_test();
        let bytes = save::encode(&g.world, None, None, &g.blocks, &g.cables, &g.members, &g.springs, &g.mech);
        let l = save::decode(&bytes).unwrap();
        assert_eq!(l.mech.gears.len(), g.mech.gears.len());
        assert_eq!(l.mech.rods.len(), 1);
        assert_eq!(l.mech.sliders.len(), 1);
        assert_eq!(l.mech.axles.len(), g.mech.axles.len());
        assert_eq!(l.mech.gears[0].motor.map(|m| m.dir), g.mech.gears[0].motor.map(|m| m.dir));
        assert_eq!(l.mech.rods[0].len, g.mech.rods[0].len);
        assert!(matches!(l.mech.gears[0].mount, Anchor::Ground(_) | Anchor::Free(_) | Anchor::Block { .. }));
    }

    #[test]
    fn undo_puts_the_world_back_as_it_was_before_each_brick() {
        let mut g = Game::new();
        let sand_before = g.world.raw(10, 10);
        g.checkpoint();
        g.blocks.push(Block::new(BlockKind::Brick, vec2(100.0, 50.0), 0.0));
        g.world.set(10, 10, Mat::Sand);
        g.world.clock += 500;
        g.checkpoint();
        g.blocks.push(Block::new(BlockKind::Long, vec2(200.0, 50.0), 0.0));
        g.world.set(11, 10, Mat::Sand);

        g.undo();
        assert_eq!(g.blocks.len(), 1, "second brick should be gone");
        assert_eq!(g.world.raw(10, 10), Mat::Sand, "first brick's world is kept");
        assert_eq!(g.world.raw(11, 10), sand_before, "later changes are undone");

        g.undo();
        assert!(g.blocks.is_empty());
        assert_eq!(g.world.raw(10, 10), sand_before);
        g.undo();
        assert!(g.toast.is_some(), "nothing left to undo");
    }

    #[test]
    fn unlimited_materials_can_be_chosen_separately_for_editing_and_simulation() {
        let mut g = Game::new();
        g.unlimited = true;
        g.unlimited_sim = false;
        assert!(g.free_now(), "editing is free");
        g.set_running(true);
        assert!(!g.free_now(), "the simulation is limited");
        g.running = false;
        g.unlimited_sim = true;
        g.set_running(true);
        assert!(g.free_now(), "the simulation was made free while editing");
    }

    #[test]
    fn the_cursor_reports_missing_materials_for_every_building_tool() {
        let mut g = Game::new();
        g.empty_world();
        g.unlimited = false;
        g.census = g.world.census();
        let at = vec2(300.0, 200.0);
        for tool in [Tool::Block(BlockKind::Wheel), Tool::Cable(CableMat::Steel), Tool::Spring(SpringKind::Coil), Tool::Beam(MemberMat::Iron)] {
            g.tool = tool;
            let m = g.missing_for_tool(at);
            assert!(m.is_some(), "an empty world has nothing to build with ({:?})", m);
            let (_, need, have) = m.unwrap();
            assert!(need > have);
        }
        g.tool = Tool::Mat(Mat::Sand);
        assert!(g.missing_for_tool(at).is_none(), "painting never costs anything");
        // With iron on screen the wheel is affordable; with unlimited materials everything is.
        for x in 0..40 {
            for y in 0..40 {
                g.world.set(x, y, Mat::Iron);
            }
        }
        g.census = g.world.census();
        g.tool = Tool::Block(BlockKind::Wheel);
        assert!(g.missing_for_tool(at).is_none());
        g.unlimited = true;
        g.tool = Tool::Cable(CableMat::Steel);
        assert!(g.missing_for_tool(at).is_none());
    }

    #[test]
    fn an_explosion_shakes_the_camera_which_then_settles() {
        let mut g = Game::new();
        g.empty_world();
        assert_eq!(g.shake, 0.0);
        g.set_running(true);
        g.world.explode(300, 300, 16);
        g.sim_step();
        assert!(g.shake > 5.0, "a blast should shake the view, got {}", g.shake);
        for _ in 0..120 {
            g.update(1.0 / 60.0);
        }
        assert_eq!(g.shake, 0.0, "the shaking dies down");
    }

    #[test]
    fn glass_breaks_in_three_stages() {
        let mut g = Game::new();
        g.empty_world();
        g.blocks.push(Block::new(BlockKind::Glass, vec2(200.0, 100.0), 0.0));
        g.hammer(vec2(200.0, 100.0));
        assert_eq!(g.blocks.len(), 2, "stage 1: two halves");
        assert!(g.blocks.iter().all(|b| b.kind == BlockKind::GlassHalf));
        let at = g.blocks[0].pos;
        g.blocks[0].hurt = 0;
        g.hammer(at);
        assert_eq!(g.blocks.len(), 5, "stage 2: one half becomes four chips");
        assert_eq!(g.blocks.iter().filter(|b| b.kind == BlockKind::GlassChip).count(), 4);
        let chip = g.blocks.iter().find(|b| b.kind == BlockKind::GlassChip).unwrap().pos;
        for b in &mut g.blocks {
            b.hurt = 0;
        }
        g.hammer(chip);
        assert_eq!(g.blocks.len(), 4, "stage 3: the chip turns into shards");
        let shards = g.world.cells.iter().filter(|c| c.m == Mat::Shard).count();
        assert!(shards > 0, "shards should be left behind");
    }

    #[test]
    fn undo_works_while_the_simulation_runs() {
        let mut g = Game::new();
        g.set_running(true);
        g.checkpoint();
        g.blocks.push(Block::new(BlockKind::Cube, vec2(300.0, 100.0), 0.0));
        g.undo();
        assert!(g.blocks.is_empty());
        assert!(g.phys.is_some(), "physics is rebuilt when running");
    }
}
