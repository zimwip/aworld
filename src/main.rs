mod backdrop;
mod blocks;
mod camera;
mod car;
mod fireworks;
mod fps;
mod firetruck;
mod kid;
mod debris;
mod dynamite;
mod mech;
mod metrics;
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
mod simthread;
mod simview;
mod science;
mod sound;
mod ui;
mod whale;
mod wolves;
mod world;

use blocks::{anchor_world, block_at, magnet, node_at, placement_ok, stamp_all, Anchor, Block, BlockKind, Cable, CableMat, Member, MemberMat, MemberRole, Physics, Spring, SpringKind, MEMBER_MAX_LEN, SPRING_MAX_LEN};
use car::{collide_cars, Car, Fate, ModelKind, Tuning};
use macroquad::prelude::*;
use render::{RenderMode, cable_preview, draw_block, draw_block_shape, draw_guides, draw_booms, draw_cable, draw_car, draw_flag, draw_member, draw_spring, draw_spring_between, Boom, LiveBolt, Renderer, View, BOLT_LIFE};
use science::{info, mat_color, palette_info, MATTERS, RECIPES};
use mech::{GearSize, Mechanism};
use mechgame::MechDraft;
use sound::{Ev, Mix, Sounds};
use ui::{draw_icon, rrect, Drawer, Icon, Ui, ACCENT, BG, BLUE, DIM, GREEN, PANEL, RED, TEXT, TILE, TILE_HOT};
use world::{Biome, Mat, PaintRule, Season, World, H, W};

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
    Electric,
    /// The liquids' physics laid bare: classes, zones, velocities, drops, the card's band.
    Simulation,
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
    /// Plants a whole tree (the wood matter's second state in the palette).
    Tree,
    Block(BlockKind),
    Cable(CableMat),
    Spring(SpringKind),
    /// A rod: its material, and whether it is deck (cars ride it) or girder (they pass through).
    Beam(MemberMat, MemberRole),
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
    Tunnel,
    Dynamite,
    Flame,
    Lightning,
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

/// Everything needed to put the world back as it was: just before a brick was dropped (undo), or
/// as it was built before the simulation started (the plan).
struct Snapshot {
    state: world::State,
    blocks: Vec<Block>,
    cables: Vec<Cable>,
    members: Vec<Member>,
    springs: Vec<Spring>,
    mech: Mechanism,
    cars: Vec<Car>,
    sticks: Vec<dynamite::Stick>,
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
    /// The world as it was built, taken when the simulation is first started from the drawing
    /// board: Réessayer puts it back. None while building (nothing has run since the last
    /// Réessayer). Whatever happens once it is taken, paused or not, is a test and is not kept.
    plan: Option<Snapshot>,
    /// How many undo steps there were when the plan was taken (later ones belong to the test).
    plan_undo: usize,
    /// The challenge (mode jeu): building is locked until Réessayer.
    challenge: bool,
    mech: Mechanism,
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
    /// The wolves howling on full-moon nights.
    wolves: wolves::Wolves,
    fireworks: fireworks::Fireworks,
    /// The little boy who comes for a walk now and then.
    kids: kid::Kids,
    /// The night (day number) whose fireworks were explained already.
    fireworks_told: u32,
    /// The day pill (the world's menu opens under it).
    hud_hit: Rect,
    /// The world's settings (sun, escape to space) and its energy budget, under the day pill.
    climate_menu: bool,
    /// The left palette hides like the views on the right; pinned, it stays out (and the picture
    /// beside it).
    panel: Drawer,
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
    /// Bolts of lightning on the screen, and thunder on its way: (seconds left, loudness).
    bolts: Vec<LiveBolt>,
    thunder: Vec<(f32, f32)>,
    /// Pieces of exploded cars, flying and bouncing.
    debris: debris::Debris,
    /// Sticks of dynamite dropped in the world, their fuses burning while it runs.
    dynamite: dynamite::Dynamite,
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
    /// Deck or girder, picked per rod material from the little arrow of its button.
    beam_role: [MemberRole; MemberMat::ALL.len()],
    /// The rod material whose deck / girder strip is open.
    beam_menu: Option<usize>,
    /// The views on the right edge, a side menu like the palette.
    views: Drawer,
    /// The motor's remote control (or the car settings), a side menu of its own under the views:
    /// its tab shows while a motor or a car is chosen.
    remote_menu: Drawer,
    /// Ctrl + wheel / Ctrl + drag zoom and move the view; Escape brings back the whole world.
    camera: camera::Camera,
    /// Camera shake in pixels (explosions), and frame-rate watching for auto speed.
    shake: f32,
    frame_ema: f32,
    slow_frames: u32,
    /// The frame counter (F3).
    fps: fps::Fps,
    /// The metrics panel (Ctrl+D): where the time of a step goes.
    metrics: metrics::Metrics,
    /// Simulation steps owed to the clock: the world runs 60 steps a second (times the speed)
    /// whatever the screen's refresh rate.
    sim_acc: f32,
    /// What the world sounds like, from its last update.
    mix: Mix,
    /// The load on each block's welds (the stress view), when the physics is in another thread.
    stress_seen: Option<std::collections::HashMap<u32, f32>>,
    /// The simulation view: its panel, its layers and its step by step.
    simview: simview::SimView,
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
        Self::with_world(world, entry, exit)
    }

    /// A game around `world`, as it starts.
    fn with_world(world: World, entry: (i32, i32), exit: (i32, i32)) -> Self {
        Game {
            science_fresh: false,
            sounds: Sounds::silent(),
            heat_cells: 0,
            undo: vec![],
            plan: None,
            plan_undo: 0,
            challenge: false,
            mech: Mechanism::default(),
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
            wolves: wolves::Wolves::new(),
            fireworks: fireworks::Fireworks::new(),
            kids: kid::Kids::new(),
            fireworks_told: 0,
            hud_hit: Rect::new(-10.0, -10.0, 0.0, 0.0),
            climate_menu: false,
            panel: Drawer { open: 1.0, pin: false, peek: 0.0 },
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
            bolts: vec![],
            thunder: vec![],
            debris: debris::Debris::default(),
            dynamite: dynamite::Dynamite::default(),
            time: 0.0,
            fullscreen: false,
            mode: ViewMode::Normal,
            paint_mode: PaintMode::Add,
            only_target: Mat::Empty,
            matter_state: MATTERS.map(|m| m.default),
            matter_menu: None,
            beam_role: [MemberRole::Deck; MemberMat::ALL.len()],
            beam_menu: None,
            views: Drawer::new(false),
            remote_menu: Drawer::new(false),
            camera: camera::Camera::default(),
            shake: 0.0,
            frame_ema: 0.016,
            slow_frames: 0,
            fps: fps::Fps::new(),
            metrics: metrics::Metrics::new(),
            sim_acc: 0.0,
            mix: Mix::default(),
            stress_seen: None,
            simview: simview::SimView::new(),
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
        let s = self.capture();
        self.undo.push(s);
        if self.undo.len() > 15 {
            self.undo.remove(0);
            self.plan_undo = self.plan_undo.saturating_sub(1);
        }
    }

    /// Everything about the world right now, without the physics.
    fn capture(&self) -> Snapshot {
        Snapshot {
            state: self.world.capture(),
            blocks: self.blocks.iter().map(|b| Block { body: None, ..b.clone() }).collect(),
            cables: self.cables.iter().map(Cable::fresh).collect(),
            members: self.members.iter().map(Member::fresh).collect(),
            springs: self.springs.iter().map(Spring::fresh).collect(),
            mech: self.mech.clone(),
            cars: self.cars.clone(),
            sticks: self.dynamite.sticks.clone(),
            arrived: self.arrived,
            lost: self.lost,
        }
    }

    fn undo(&mut self) {
        if self.challenge {
            self.say("Défi en cours : Réessayer pour reprendre la construction");
            return;
        }
        // During a test, undo stops at the plan: the building steps come back with Réessayer.
        if self.plan.is_some() && self.undo.len() <= self.plan_undo {
            self.say("Rien à annuler dans ce test (Réessayer revient au départ)");
            return;
        }
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
        self.dynamite.sticks = s.sticks;
        self.arrived = s.arrived;
        self.lost = s.lost;
        self.cable_from = None;
        self.cable_via.clear();
        self.booms.clear();
        self.bolts.clear();
        self.thunder.clear();
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
            // Only the first start from the drawing board takes the plan: a pause and a new start
            // go on with the same test.
            if self.plan.is_none() {
                self.plan = Some(self.capture());
                self.plan_undo = self.undo.len();
                self.arrived = 0;
                self.lost = 0;
            }
            if self.phys.is_none() {
                self.build_physics();
            }
        }
        self.running = run;
    }

    /// Is the world being built (nothing has run since the last Réessayer)?
    fn building(&self) -> bool {
        self.plan.is_none()
    }

    /// The challenge: back to the plan if a test was going on, building locked, and go.
    fn start_challenge(&mut self) {
        if self.plan.is_some() {
            self.reset();
        }
        self.set_running(true);
        self.challenge = true;
        self.shovel = None;
        self.selection = None;
        self.say("Défi lancé : la construction est verrouillée. Réessayer pour la reprendre.");
    }

    /// Réessayer: the world as it was built, the counters at zero, back to building.
    fn reset(&mut self) {
        let Some(plan) = self.plan.take() else {
            self.say("Tu es déjà au départ");
            return;
        };
        self.world.restore_state(&plan.state);
        self.sensors.clear_history(&self.world);
        self.blocks = plan.blocks;
        self.cables = plan.cables;
        self.members = plan.members;
        self.springs = plan.springs;
        self.mech = plan.mech;
        self.cars = plan.cars;
        self.dynamite.sticks = plan.sticks;
        self.undo.truncate(self.plan_undo);
        self.mech_draft = None;
        self.spring_draft = None;
        self.cable_from = None;
        self.cable_via.clear();
        self.member_from = None;
        self.selection = None;
        self.drag = None;
        self.phys = None;
        self.booms.clear();
        self.bolts.clear();
        self.thunder.clear();
        self.debris.clear();
        self.running = false;
        self.challenge = false;
        self.arrived = 0;
        self.lost = 0;
        self.refresh_mask();
    }

    fn fresh(&mut self) {
        self.undo.clear();
        self.blocks.clear();
        self.cables.clear();
        self.members.clear();
        self.springs.clear();
        self.mech = Mechanism::default();
        self.plan = None;
        self.challenge = false;
        self.mech_draft = None;
        self.spring_draft = None;
        self.cable_from = None;
        self.cable_via.clear();
        self.member_from = None;
        self.phys = None;
        self.cars.clear();
        self.dynamite.clear();
        self.debris.clear();
        self.selected_car = None;
        self.running = false;
        self.arrived = 0;
        self.lost = 0;
        self.refresh_mask();
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
            self.cars.push(car);
        }
        self.set_running(true);
    }

    /// Saves the plan, never a world half way through a test.
    fn save(&mut self) {
        let bytes = match &self.plan {
            Some(p) => save::encode_cells(self.world.biome, p.state.clock(), |x, y| World::raw_in(&p.state, x, y), self.entry, self.exit, &p.blocks, &p.cables, &p.members, &p.springs, &p.mech),
            None => save::encode(&self.world, self.entry, self.exit, &self.blocks, &self.cables, &self.members, &self.springs, &self.mech),
        };
        match save::save_slot(self.slot, &bytes) {
            Ok(()) if self.plan.is_some() => self.say(format!("Monde de départ sauvegardé (emplacement {})", self.slot + 1)),
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
        // Free on whole cells; pulled onto nearby blocks unless Shift is held.
        let pos = if is_key_down(KeyCode::LeftShift) { at.round() } else { magnet(&self.blocks, kind, angle, at) };
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

    fn beam_click(&mut self, mat: MemberMat, role: MemberRole, at: Vec2) {
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
                let units = mat.cost(len, role);
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
                self.members.push(Member::new(mat, a, b, p0, p1).with_role(role));
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

    // ----- dynamite ---------------------------------------------------------

    /// A stick of dynamite at `at`: it falls and lies on the ground, or stays planted in the
    /// matter it was pushed into. Its fuse burns while the simulation runs.
    fn drop_dynamite(&mut self, at: Vec2) {
        if self.dynamite.sticks.len() >= dynamite::MAX_STICKS {
            self.say(format!("Pas plus de {} bâtons de dynamite à la fois", dynamite::MAX_STICKS));
            return;
        }
        self.checkpoint();
        self.dynamite.drop(&self.world, at);
        if !self.running {
            self.dynamite.settle(&self.world);
            if self.dynamite.sticks.len() == 1 {
                self.say("Dynamite posée : sa mèche s'allumera quand la simulation tournera");
            }
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
        if (is_key_pressed(KeyCode::Delete) || is_key_pressed(KeyCode::Backspace)) && self.tool == Tool::Select && !self.challenge {
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
        // In the simulation view, "." takes one step and shows what it changed.
        if self.mode == ViewMode::Simulation && is_key_pressed(KeyCode::Period) {
            self.step_by_step(1);
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
        if is_key_pressed(KeyCode::K) && !self.kids.summon(&self.world) && self.kids.kid.is_none() {
            self.say("Il faut un bout de terre ferme pour se promener");
        }
        if is_key_pressed(KeyCode::W) {
            if self.whales.summon(&self.world) {
                self.say("Une baleine remonte respirer !");
            } else if self.whales.whale.is_none() {
                self.say("Il faut une grande étendue d'eau profonde pour une baleine");
            }
        }
        if is_key_pressed(KeyCode::L) {
            let night = self.world.sun() <= 0.0 && self.world.moon_arc().is_some();
            if self.world.full_moon() && night {
                if self.wolves.summon(&self.world) {
                    self.say("Les loups arrivent sous la pleine lune...");
                }
            } else {
                let n = self.world.nights_to_full_moon();
                self.say(match n {
                    0 => format!("Les loups attendent que la pleine lune se lève ({})", self.world.moon_name()),
                    1 => format!("Les loups attendent la pleine lune : demain soir ! ({} ce soir)", self.world.moon_name()),
                    _ => format!("Les loups attendent la pleine lune : encore {n} nuits ({} ce soir)", self.world.moon_name()),
                });
            }
        }
        if is_key_pressed(KeyCode::Y) {
            self.mode = if self.mode == ViewMode::Humidity { ViewMode::Normal } else { ViewMode::Humidity };
        }
        if is_key_pressed(KeyCode::P) {
            self.mode = if self.mode == ViewMode::Pressure { ViewMode::Normal } else { ViewMode::Pressure };
        }
        if is_key_pressed(KeyCode::E) {
            self.mode = if self.mode == ViewMode::Electric { ViewMode::Normal } else { ViewMode::Electric };
        }
        if is_key_pressed(KeyCode::F11) || is_key_pressed(KeyCode::F) {
            self.toggle_fullscreen();
        }
        if is_key_pressed(KeyCode::F3) {
            self.fps.shown = !self.fps.shown;
        }
        // Ctrl+D: the debug panel, where the time of a step goes (see `metrics.rs`).
        if ctrl_down() && is_key_pressed(KeyCode::D) {
            self.metrics.shown = !self.metrics.shown;
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
                self.tool = matter_tool(MATTERS[i].states[self.matter_state[i]]);
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
        let over_ui = ui.over_ui();
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
        let over_ui = ui.over_ui();
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
        // A click on the little boy is for him, not for the tool.
        if click && self.kids.kid.as_ref().map_or(false, |k| k.contains(at)) {
            self.kids.ouch(kid::Hit::Click);
            self.last = None;
            return;
        }
        // During the challenge nothing is built: the probes still go down, and the flamethrower and
        // the lightning still put the construction to the test (the right button, which erases, does not).
        let trial = matches!(self.tool, Tool::Sensor | Tool::Flame | Tool::Lightning);
        if self.challenge && (!trial || right) {
            if click || rclick {
                self.say("Défi en cours : la construction est verrouillée (Réessayer pour la reprendre)");
            }
            return;
        }

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
                if let Some(i) = self.members.iter().position(|m| m.distance_to(at) < m.thickness() / 2.0 + 3.0) {
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
                if click && m == Mat::Wood {
                    // Each stroke of wood is a log of its own.
                    self.world.new_log();
                }
                self.world.line_rule(from, g, self.brush, m, rule);
            }
            Tool::Tree if click => {
                if !self.world.plant_tree(g.0, g.1, self.brush as f32 / 20.0) {
                    self.say("Pas de place pour un arbre ici : il lui faut un sol dégagé.".to_string());
                }
            }
            Tool::Block(k) if click => self.place_block(k, at),
            Tool::Cable(m) if click => self.cable_click(m, at),
            Tool::Spring(k) if click => self.spring_click(k, at),
            Tool::Beam(m, role) if click => self.beam_click(m, role, at),
            Tool::Gear(_) | Tool::Motor | Tool::Axle | Tool::Rail | Tool::Crank | Tool::Pivot | Tool::Screw | Tool::Winch | Tool::Pulley if click => self.mech_click(at),
            Tool::Pipe | Tool::Valve | Tool::Pump if click => self.fluid_click(at),
            Tool::Shovel => self.shovel_mouse(at, click, left),
            Tool::Select => self.select_mouse(at, click, left, is_key_down(KeyCode::LeftShift)),
            Tool::Tunnel => self.world.tunnel_line(from, g, self.brush.max(2)),
            Tool::Dynamite if click => self.drop_dynamite(at),
            Tool::Flame => {
                self.world.flamethrower(g.0, g.1, self.brush.max(3));
                if self.trucks.scorch(vec2(g.0 as f32, g.1 as f32), self.brush.max(3) as f32) {
                    self.blow_up_truck();
                }
            }
            Tool::Lightning if click => self.world.strike(g.0, g.1),
            Tool::Sensor if click => self.sensor_click(at),
            Tool::Entry if click => self.entry = Some(g),
            Tool::Exit if click => self.exit = Some(g),
            _ => {}
        }
    }

    // ----- simulation -------------------------------------------------------

    /// One frame of the world on a single thread (the web, `--demo`, the tests): the world's
    /// update, its steps, and the sound.
    fn update(&mut self, dt: f32) {
        self.tick_toast(dt);
        if let Some(steps) = self.update_world(dt, dt) {
            // std's clock, macroquad's in the browser (std has none there; macroquad's needs a
            // window, which the tests have not).
            let start = metrics::now();
            for _ in 0..steps {
                self.sim_step();
            }
            self.after_steps(dt, steps, metrics::now() - start);
        }
        let mix = self.mix;
        self.sounds.update(dt, &mix);
    }

    /// The message at the bottom fades out.
    fn tick_toast(&mut self, dt: f32) {
        if let Some((_, t)) = &mut self.toast {
            *t -= dt;
        }
        if self.toast.as_ref().map_or(false, |t| t.1 <= 0.0) {
            self.toast = None;
        }
    }

    /// Everything that lives in the world but its steps, for `dt` seconds: the visitors, the
    /// simplifications that keep it smooth... The steps owed to the clock, or None while paused.
    /// `pace` is how long an update takes: the frame on one thread, the work of the last update
    /// in the simulation thread (a slow screen does not slow that one down).
    /// On the desktop it runs in the simulation thread (see `simthread.rs`): it must not touch
    /// what only the screen owns (the camera, the menus); its messages and sounds go to the screen
    /// with the next picture.
    fn update_world(&mut self, dt: f32, pace: f32) -> Option<usize> {
        let around = self.metrics.start();
        self.time += dt;
        self.shake *= 0.88f32.powf(dt * 60.0);
        if self.shake < 0.15 {
            self.shake = 0.0;
        }
        // Keep the game smooth: if updates stay slow while simulating, run fewer steps each.
        self.frame_ema = self.frame_ema * 0.92 + pace * 0.08;
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
        let building = matches!(self.tool, Tool::Block(_) | Tool::Cable(_) | Tool::Spring(_) | Tool::Beam(..) | Tool::Gear(_) | Tool::Motor | Tool::Axle | Tool::Rail | Tool::Crank | Tool::Pivot | Tool::Screw | Tool::Winch | Tool::Pulley | Tool::Pipe | Tool::Valve | Tool::Pump);
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
        match self.wolves.update(dt, &self.world) {
            Some(wolves::News::Howl) => {
                self.sounds.emit(Ev::Howl);
                if self.wolves.pack.as_ref().map_or(false, |p| p.age < 4.0) {
                    self.say("Pleine lune : les loups hurlent ! Aouuuuh !");
                }
            }
            Some(wolves::News::Cub) => {
                self.sounds.emit(Ev::Yip);
                self.say("Le louveteau essaie aussi... il chante un peu faux");
            }
            Some(wolves::News::Wink) => self.say("La lune, flattée, leur fait un clin d'œil !"),
            Some(wolves::News::Fact) => self.say("En vrai, les loups ne hurlent pas à la lune : ils lèvent la tête pour que leur appel porte loin"),
            None => {}
        }
        self.fireworks.update(dt);
        self.update_kid(dt);
        if self.fireworks.take_bursts() > 0 {
            self.sounds.emit(Ev::Pop);
        }
        for b in &mut self.booms {
            b.t += dt / 0.7;
        }
        self.booms.retain(|b| b.t < 1.0);
        if !self.running {
            let (blocks, members) = (&self.blocks, &self.members);
            self.mech.follow_mounts(|a| anchor_world(a, blocks, members));
            self.refresh_reel_anchors();
            self.mix = Mix::default();
            self.metrics.around(around, true);
            return None;
        }
        // A fixed time step: 60 steps a second at x1, the same on a 60 Hz or a 144 Hz screen. A
        // machine that cannot keep up catches up a little, then lets the time go (and the frame
        // watch above simplifies the physics or lowers the speed).
        self.sim_acc += dt * STEPS_PER_SECOND * self.speed as f32;
        let owed = self.sim_acc.floor() as usize;
        let max = self.speed * 2;
        let steps = owed.min(max);
        self.sim_acc = if owed > max { 0.0 } else { self.sim_acc - steps as f32 };
        self.metrics.around(around, true);
        self.world.prof.on = self.metrics.on();
        Some(steps)
    }

    /// After the `steps` steps of an update of `dt` seconds, that took `seconds`: the profile, the
    /// lightning and what the world sounds like.
    fn after_steps(&mut self, dt: f32, steps: usize, seconds: f64) {
        let around = self.metrics.start();
        self.fps.steps(seconds, steps);
        // `AWORLD_PROFILE=1`: every 600 steps, where the time of a step went (see `metrics.rs`).
        if let Some(report) = self.metrics.take_log() {
            eprint!("--- profil sur 600 pas (vitesse x{}, simplification {})\n{report}", self.speed, self.world.lod);
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
        self.update_lightning(dt);
        self.mix = Mix {
            wind: ((self.world.wind.abs() - 0.55) / 0.45).clamp(0.0, 1.0),
            water: (self.world.take_water_flow() as f32 / (dt * 3600.0 * self.speed as f32).max(1.0)).sqrt().min(1.0),
            fire: (self.heat_cells as f32 / 150.0).sqrt().min(1.0),
            rpm,
        };
        self.metrics.around(around, false);
    }

    /// The metrics panel (Ctrl+D), with what it reads from the game.
    fn draw_metrics(&self, ui: &Ui, sw: f32, sh: f32, draw_ms: (f32, f32)) {
        if !self.metrics.shown {
            return;
        }
        let info = metrics::Info {
            awake: self.world.zones.awake_count(),
            zones: world::sleep::CW * world::sleep::CH,
            lod: self.world.lod,
            speed: self.speed,
            blocks: self.blocks.len(),
            members: self.members.len(),
            cars: self.cars.len(),
            pipes: self.mech.plumbing.pipes.len(),
            scene_ms: draw_ms.0,
            ui_ms: draw_ms.1,
        };
        self.metrics.draw(ui, sw, sh, &info);
    }

    /// New bolts of lightning go on the screen; their thunder comes a little later, the later the
    /// farther they struck from the middle of the valley (sound is slow, light is not).
    fn update_lightning(&mut self, dt: f32) {
        for b in &mut self.bolts {
            b.age += dt;
        }
        self.bolts.retain(|b| b.age < BOLT_LIFE);
        for bolt in std::mem::take(&mut self.world.bolts) {
            // Lightning that comes down through the fire engine sets its fuel off.
            if bolt.ground && self.trucks.truck.as_ref().map_or(false, |t| bolt.path.iter().any(|&(x, y)| t.touches(vec2(x, y), 2.0))) {
                self.blow_up_truck();
            }
            let end = *bolt.path.last().unwrap_or(&(0.0, 0.0));
            let far = vec2(end.0 - world::W as f32 * 0.5, end.1 - 360.0).length();
            let loud = bolt.power.max(0.3) * (1.0 - far / 900.0).clamp(0.15, 1.0) * if bolt.ground { 1.0 } else { 0.6 };
            self.thunder.push((0.15 + far / 320.0, loud));
            if bolt.ground {
                self.shake = self.shake.max(3.0 * loud);
            }
            let seed = (end.0 * 12.9898 + end.1 * 78.233).sin().abs().fract();
            if self.bolts.len() < 8 {
                self.bolts.push(LiveBolt { bolt, age: 0.0, seed });
            }
        }
        for t in &mut self.thunder {
            t.0 -= dt;
        }
        for &(_, loud) in self.thunder.iter().filter(|t| t.0 <= 0.0) {
            self.sounds.emit(Ev::Thunder(loud));
        }
        self.thunder.retain(|t| t.0 > 0.0);
    }

    /// The particles the game moves besides the world's cells, for the simulation view: the
    /// pieces of wrecked cars (cells a step) and the fireworks (cells a second), in m/s.
    fn flying_particles(&self) -> Vec<simview::Particle> {
        use world::inspect::{CELL_METRES, CELL_STEP_MS};
        let debris = self.debris.pieces.iter().filter(|p| !p.rest).map(|p| simview::Particle { x: p.p.x, y: p.p.y, vx: p.v.x * CELL_STEP_MS, vy: p.v.y * CELL_STEP_MS, kind: 0 });
        let rockets = self.fireworks.rockets.iter().map(|r| (r.pos, r.vel));
        let sparks = self.fireworks.sparks.iter().map(|s| (s.pos, s.vel));
        let fireworks = rockets.chain(sparks).map(|(p, v)| simview::Particle { x: p.x, y: p.y, vx: v.x * CELL_METRES, vy: v.y * CELL_METRES, kind: 1 });
        debris.chain(fireworks).collect()
    }

    /// The simulation view's step by step: `n` steps, the world paused after them, and what they
    /// changed kept for its diff.
    fn step_by_step(&mut self, n: usize) {
        if !self.running {
            // Takes the plan and builds the physics, as a start would.
            self.set_running(true);
        }
        self.running = false;
        self.simview.before_steps(&self.world);
        for _ in 0..n {
            self.sim_step();
        }
        self.simview.after_steps(&self.world);
    }

    fn sim_step(&mut self) {
        // Each part's time goes to the metrics panel (Ctrl+D; see `metrics.rs`).
        let mut clock = self.metrics.start();
        self.world.step();
        self.metrics.lap(metrics::WORLD, &mut clock);
        self.dynamite.step(&mut self.world);
        self.metrics.lap(metrics::DYNAMITE, &mut clock);
        self.sensors.step(&self.world);
        self.metrics.lap(metrics::SENSORS, &mut clock);
        let exit = self.exit;
        let was_driving: Vec<bool> = self.cars.iter().map(|c| c.fate == Fate::Driving).collect();
        for c in &mut self.cars {
            c.step(&self.world, exit);
        }
        self.metrics.lap(metrics::CARS, &mut clock);
        let (clang, smoke) = self.debris.step(&self.world);
        if clang > 0.5 {
            self.sounds.emit(Ev::Thud(clang * 0.5));
        }
        for (x, y) in smoke {
            if self.world.raw(x, y) == Mat::Empty {
                self.world.set(x, y, Mat::Smoke);
            }
        }
        self.metrics.lap(metrics::DEBRIS, &mut clock);
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

        self.metrics.lap(metrics::CARS, &mut clock);
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
            for (at, hard) in p.take_bumps() {
                self.sounds.emit(Ev::Thud(hard));
                if self.kids.kid.as_ref().map_or(false, |k| (k.centre() - at).length() < 16.0) {
                    self.kids.ouch(kid::Hit::Block);
                }
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
        self.metrics.lap(metrics::BLOCKS, &mut clock);
        self.mech_step();
        self.metrics.lap(metrics::MECH, &mut clock);
        self.fluid_step();
        self.metrics.lap(metrics::PIPES, &mut clock);
        stamp_all(&mut self.world, &self.blocks, &self.members);
        self.metrics.lap(metrics::STAMPS, &mut clock);

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
        let mut truck_hit = false;
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
            if self.kids.kid.as_ref().map_or(false, |k| (k.centre() - c).length() < r as f32 * 1.6 + 12.0) {
                self.kids.ouch(kid::Hit::Blast);
            }
            for car in &mut self.cars {
                if (car.p - c).length() < r as f32 * 1.4 && car.fate == Fate::Driving {
                    car.fate = Fate::Exploded;
                }
            }
            truck_hit |= self.trucks.truck.as_ref().map_or(false, |t| t.touches(c, r as f32 * 0.8));
            self.sounds.emit(Ev::Boom(r as f32));
            self.booms.push(Boom { pos: c, t: 0.0, color: Color::new(1.0, 0.5, 0.1, 1.0) });
            // The pressure wave rattles the whole picture.
            self.shake = self.shake.max((r as f32 * 0.75).min(18.0));
        }
        // A blast that reaches the fire engine sets it off too (its own goes off next step).
        if truck_hit {
            self.blow_up_truck();
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
        self.metrics.lap(metrics::AFTERMATH, &mut clock);
        let news = self.trucks.step(&mut self.world);
        self.metrics.lap(metrics::TRUCKS, &mut clock);
        match news {
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
        self.metrics.lap(metrics::AFTERMATH, &mut clock);
        self.metrics.end_step(&mut self.world.prof);
    }

    /// The fire engine goes up: a bigger bang than a car's, its pieces and ladder flying, and the
    /// water left in its tank bursting out all around.
    fn blow_up_truck(&mut self) {
        let Some(t) = self.trucks.blow_up() else { return };
        let c = vec2(t.x, t.y - firetruck::BODY_HEIGHT * 0.5);
        self.world.explode(c.x as i32, c.y as i32, 16);
        self.debris.burst_truck(&t);
        self.booms.push(Boom { pos: c, t: 0.0, color: Color::new(1.0, 0.75, 0.2, 1.0) });
        self.say("BOUM ! Le camion de pompiers a explosé, et sa citerne arrose tout autour");
    }

    /// The little boy: what he can see for his remarks, and the cars that run into him.
    fn update_kid(&mut self, dt: f32) {
        let at = self.kids.kid.as_ref().map(|k| k.centre());
        let sights = kid::Sights {
            whale: self.whales.whale.as_ref().map_or(false, |w| w.sink() < 0.0),
            plane: self.planes.plane.is_some(),
            truck: self.trucks.truck.is_some(),
            fireworks: self.fireworks.active(),
            cars: self.cars.iter().filter(|c| c.fate == Fate::Driving).count(),
            blocks: self.blocks.len(),
            night: backdrop::daylight(self.world.phase()) < 0.3,
            temp_c: at.map_or(15.0, |p| self.world.temp_c(p.x as i32, p.y as i32)),
        };
        self.kids.update(dt, &self.world, &sights);
        if let Some(p) = at {
            let car = self.cars.iter().any(|c| c.fate == Fate::Driving && (c.p - p).length() < 14.0 && c.v.length() > 0.3);
            let truck = self.trucks.truck.as_ref().map_or(false, |t| (vec2(t.x, t.y - 8.0) - p).length() < 18.0);
            if car || truck {
                self.kids.ouch(kid::Hit::Car);
            }
        }
    }

    // ----- drawing ----------------------------------------------------------

    /// The just-for-fun visitors (Beluga, whale, fire engine...), drawn over the world in every
    /// view mode, not only the normal one. A new cameo is drawn here.
    fn draw_cameos(&self, view: View) {
        self.planes.draw(view, self.time);
        self.whales.draw(view);
        self.wolves.draw(view);
        self.trucks.draw(view, self.time);
        self.kids.draw(view, self.time);
    }

    fn draw_scene(&mut self, view: View, renderer: &mut Renderer, ui: &mut Ui) {
        let phase = self.world.phase();
        let day = backdrop::sky_light(phase);
        let thermal = self.mode == ViewMode::Thermal;
        // The thermal and pressure views paint the whole picture themselves.
        let flat = thermal || matches!(self.mode, ViewMode::Pressure | ViewMode::Humidity | ViewMode::Electric | ViewMode::Simulation);
        let render_mode = match self.mode {
            ViewMode::Thermal => RenderMode::Thermal,
            ViewMode::Pressure => RenderMode::Pressure,
            ViewMode::Humidity => RenderMode::Humidity,
            ViewMode::Electric => RenderMode::Electric,
            ViewMode::Stress => RenderMode::Stress,
            _ => RenderMode::Normal,
        };
        if !flat {
            let moon = backdrop::Moon { arc: self.world.moon_arc(), age: self.world.moon_age(), light: self.world.moon_light(), wink: self.wolves.wink() };
            backdrop::draw_sky(view, self.world.biome, phase, self.world.year_phase(), self.world.wind, self.time, self.world.season(), moon);
        }
        if self.mode == ViewMode::Simulation {
            self.simview.draw_back(&self.world, view);
        } else {
            renderer.draw_world(&self.world, view, render_mode, self.time);
        }
        if !flat {
            renderer.draw_gas(&self.world, view);
        }
        // The cameos show in every view, the thermal, pressure and humidity ones included.
        self.draw_cameos(view);
        let stress = if self.mode == ViewMode::Stress { self.phys.as_ref().map(|p| p.stress(&self.blocks)).or_else(|| self.stress_seen.clone()) } else { None };
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
        // The push of the liquids on each solid body: an arrow where it bears, as long as the
        // square root of the force, and the force (kN per metre of depth).
        if self.mode == ViewMode::Stress {
            for p in &self.world.pushes {
                let f = (p.fx * p.fx + p.fy * p.fy).sqrt();
                // (The steps of a slanting face each get a little push: only the faces that count.)
                if f < 20.0 {
                    continue;
                }
                let a = view.to_screen(vec2(p.x, p.y));
                let len = (f.sqrt() * 6.0).clamp(14.0, 160.0);
                let dir = vec2(p.fx, p.fy) / f;
                let b = a + dir * len;
                let col = Color::new(0.95, 0.35, 0.95, 0.95);
                draw_line(a.x, a.y, b.x, b.y, 3.0, col);
                let side = vec2(-dir.y, dir.x) * 6.0;
                draw_triangle(b + dir * 4.0, b - dir * 8.0 + side, b - dir * 8.0 - side, col);
                ui.text(&format!("{f:.0} kN/m"), b.x + 6.0, b.y - 6.0, 13, col);
            }
        }
        let over_ui_now = ui.over_ui();
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
        self.dynamite.draw(view, self.running, self.time);
        // Clods thrown out of a crater, in flight.
        let size = view.s.max(1.0);
        for p in &self.world.ejecta {
            let s = view.to_screen(vec2(p.x, p.y));
            draw_rectangle(s.x - size * 0.5, s.y - size * 0.5, size, size, mat_color(p.m));
        }
        // Drops of liquid in flight (spray, splashes): the more liquid, the bigger.
        for d in self.world.liquid_drops() {
            let s = view.to_screen(vec2(d.x, d.y));
            let r = size * (0.35 + 0.65 * (d.q as f32 / 65536.0).min(1.0).sqrt());
            draw_rectangle(s.x - r * 0.5, s.y - r * 0.5, r, r, mat_color(d.m));
        }
        draw_booms(&self.booms, view);
        if !flat {
            renderer.draw_night(view, day, self.time);
        } else if thermal {
            self.draw_sun_on_thermal(view, ui);
        }
        // Fireworks and lightning are light: drawn over the night, so they glow in the dark.
        self.fireworks.draw(view, self.time);
        self.wolves.draw_song(view, ui);
        if !flat {
            renderer.draw_corona(&self.world, view, self.time);
        }
        renderer.draw_bolts(&self.bolts, view);
        if self.mode == ViewMode::Wind {
            self.draw_wind_field(view);
        }
        if self.mode == ViewMode::Simulation {
            let particles = self.flying_particles();
            self.simview.draw_layers(&self.world, view, ui, &particles);
        }
        // What the little boy says, over the night and the science views so it can be read.
        self.kids.draw_bubble(view, ui);

        // Cursor preview.
        let over_ui = ui.over_ui();
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
        if let (Tool::Beam(mat, role), Some((a, p0))) = (self.tool, self.member_from) {
            let at = if inside { view.to_world(ui.mouse) } else { p0 };
            let (_, p1) = self.beam_end(p0, at, &a);
            let len = (p1 - p0).length();
            let (cr, cg, cb) = mat.color();
            let ok = len >= 12.0;
            let col = if ok { Color::from_rgba(cr, cg, cb, 255) } else { Color::new(1.0, 0.3, 0.3, 1.0) };
            let (sa, sb) = (view.to_screen(p0), view.to_screen(p1));
            draw_line(sa.x, sa.y, sb.x, sb.y, role.thickness(mat) * view.s, Color::new(col.r, col.g, col.b, 0.6));
            draw_circle(sa.x, sa.y, 5.0, WHITE);
            draw_circle(sb.x, sb.y, 5.0, WHITE);
            let label = format!("{:.0} / {:.0}", len, MEMBER_MAX_LEN);
            ui.text(&label, (sa.x + sb.x) / 2.0 + 8.0, (sa.y + sb.y) / 2.0 - 8.0, 16, WHITE);
        }
        if inside && matches!(self.tool, Tool::Cable(_) | Tool::Spring(_) | Tool::Beam(..)) {
            self.draw_anchor_hint(ui, view, view.to_world(ui.mouse));
        }
        if inside {
            let at = view.to_world(ui.mouse);
            match self.tool {
                Tool::Block(k) => {
                    let cand = self.candidate(k, at);
                    let ok = placement_ok(&self.world, &self.blocks, &cand, None) && self.missing_for_tool(at).is_none();
                    let tint = if ok { Color::new(0.3, 1.0, 0.5, 1.0) } else { Color::new(1.0, 0.1, 0.1, 1.0) };
                    draw_guides(&blocks::guides(&self.blocks, &cand), view);
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
                Tool::Tunnel => {
                    // The hole, and the brick vault drawn over its top half.
                    let r = (self.brush.max(2) as f32 + 0.5) * view.s;
                    draw_circle_lines(ui.mouse.x, ui.mouse.y, r, 1.5, Color::new(1.0, 1.0, 1.0, 0.9));
                    let vault = r + (world::TUNNEL_LINING as f32 + 0.5) * view.s;
                    let brick = Color::new(0.85, 0.42, 0.3, 0.95);
                    let n = 16;
                    for k in 0..n {
                        let (a0, a1) = (std::f32::consts::PI * (1.0 + k as f32 / n as f32), std::f32::consts::PI * (1.0 + (k + 1) as f32 / n as f32));
                        draw_line(ui.mouse.x + vault * a0.cos(), ui.mouse.y + vault * a0.sin(), ui.mouse.x + vault * a1.cos(), ui.mouse.y + vault * a1.sin(), 2.5, brick);
                    }
                }
                Tool::Beam(..) => {
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
                Tool::Dynamite => {
                    let p = vec2(at.x - dynamite::LEN as f32 / 2.0, at.y - dynamite::THICK as f32 / 2.0).round();
                    dynamite::draw_stick(view, p, 1.0, false, self.time, 0.55);
                }
                Tool::Flame => draw_circle_lines(ui.mouse.x, ui.mouse.y, self.brush.max(3) as f32 * view.s, 1.5, RED),
                Tool::Lightning => {
                    draw_circle_lines(ui.mouse.x, ui.mouse.y, 5.0, 1.5, Color::new(1.0, 0.92, 0.4, 0.9));
                }
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
        if w.sun() > 0.0 {
            let (sx, sy) = w.sun_position();
            let c = view.to_screen(vec2(sx, sy));
            for (r, a) in [(64.0, 0.10), (44.0, 0.18), (28.0, 0.32), (16.0, 0.9)] {
                draw_circle(c.x, c.y, r * view.s.max(0.8), Color::new(1.0, 0.92 - r * 0.002, 0.55, a));
            }
            draw_circle_lines(c.x, c.y, 16.0 * view.s.max(0.8), 2.0, WHITE);
            ui.text("Soleil", c.x + 24.0, c.y - 20.0, 15, WHITE);
        } else if let Some((mx, my)) = w.moon_position() {
            let c = view.to_screen(vec2(mx, my));
            draw_circle(c.x, c.y, 12.0 * view.s.max(0.8), Color::new(0.75, 0.82, 1.0, 0.8));
            ui.text("Lune", c.x + 18.0, c.y - 14.0, 14, Color::new(0.85, 0.9, 1.0, 1.0));
        }
    }

    fn draw_hud(&mut self, ui: &mut Ui, view: View) {
        let r = view.visible();
        let w = &self.world;
        let night = w.sun() <= 0.0;
        let arrow = if w.wind > 0.08 { "→" } else if w.wind < -0.08 { "←" } else { "·" };
        let mut text = format!(
            "{}  jour {}  ·  {}  ·  {:.0} °C  ·  vent {} {:.0} km/h",
            w.clock_label(),
            w.day_number(),
            w.season().name(),
            w.surface_temp_c(),
            arrow,
            w.wind.abs() * 40.0
        );
        // At night, tonight's moon.
        if night {
            text += &format!("  ·  {}", w.moon_name());
        }
        let tw = ui.text_w(&text, 14) + 38.0;
        let pill = Rect::new(r.x + 8.0, r.y + 8.0, tw, 26.0);
        self.hud_hit = pill;
        ui.layer("hud");
        ui.claim("hud", pill);
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
        } else if ui.pressed && !self.quit_modal && pill.contains(ui.mouse) {
            // Anywhere else on the pill opens (or closes) the world's settings.
            self.climate_menu = !self.climate_menu;
        } else if ui.hover(pill) {
            self.hover_info = Some(("Le monde".to_string(), "Clique pour régler le soleil et voir le bilan d'énergie.".to_string()));
        }

        let stats = format!("Arrivées {}   Perdues {}", self.arrived, self.lost);
        let sw = ui.text_w(&stats, 14) + 22.0;
        let sp = Rect::new(r.x + 8.0, r.y + r.h - 32.0, sw, 24.0);
        rrect(sp, 12.0, Color::new(0.0, 0.0, 0.0, 0.5));
        ui.text(&stats, sp.x + 11.0, sp.y + 17.0, 14, WHITE);

        let (label, col) = match (self.building(), self.challenge, self.running) {
            (true, _, _) => ("CONSTRUCTION", BLUE),
            (false, true, true) => ("DÉFI", RED),
            (false, true, false) => ("DÉFI (pause)", RED),
            (false, false, true) => ("TEST", ORANGE),
            (false, false, false) => ("TEST (pause)", ORANGE),
        };
        let bw = ui.text_w(label, 14) + 32.0;
        let gap = 92.0;
        rrect(Rect::new(r.x + r.w - bw - 8.0 - gap, r.y + 8.0, bw, 26.0), 13.0, Color::new(0.0, 0.0, 0.0, 0.5));
        draw_circle(r.x + r.w - bw + 6.0 - gap, r.y + 21.0, 5.0, col);
        ui.text(label, r.x + r.w - bw + 16.0 - gap, r.y + 26.0, 14, WHITE);
    }

    /// Under the day pill: where the world's energy comes from and goes, how much it holds over the
    /// last day, and the settings of the sun and of the escape to space.
    fn draw_climate_menu(&mut self, ui: &mut Ui) {
        let pill = self.hud_hit;
        let r = Rect::new(pill.x, pill.y + pill.h + 6.0, 640.0, 332.0);
        ui.layer("climate");
        // From under the pill, which stays clickable to close the menu.
        ui.claim("climate", Rect::new(r.x, pill.y + pill.h, r.w, r.y + r.h - pill.y - pill.h));
        rrect(r, 14.0, Color::new(0.06, 0.07, 0.13, 0.94));
        self.draw_live_weather(ui, Rect::new(r.x + 340.0, r.y, r.w - 340.0, r.h));
        let (x, w) = (r.x + 14.0, 340.0 - 28.0);
        let c = &self.world.climate;
        ui.text("Bilan d'énergie (par heure)", x, r.y + 24.0, 16, ACCENT);
        // Where the energy comes from and where it goes: one bar each, on the same scale.
        let flows = [("Soleil → monde", c.rates[0], ACCENT), ("Monde → espace", -c.rates[1], BLUE), ("Feux → monde", c.rates[2], RED)];
        let top = flows.iter().map(|f| f.1.abs()).fold(1.0f32, f32::max);
        for (k, (label, v, col)) in flows.iter().enumerate() {
            let y = r.y + 36.0 + k as f32 * 22.0;
            ui.text(label, x, y + 13.0, 13, TEXT);
            let bx = x + 118.0;
            let bw = (w - 118.0 - 64.0) * (v.abs() / top);
            rrect(Rect::new(bx, y + 4.0, bw.max(2.0), 11.0), 4.0, *col);
            ui.text(&format!("{:+.0} k", v / 1000.0), x + w - 58.0, y + 13.0, 13, DIM);
        }
        // The heat the world holds over the last day: flat when the budget balances.
        let gy = r.y + 112.0;
        let g = Rect::new(x, gy, w, 58.0);
        rrect(g, 6.0, Color::new(0.0, 0.0, 0.0, 0.35));
        ui.text("Chaleur du monde, dernières 24 h", x + 6.0, gy + 14.0, 12, DIM);
        let h: Vec<f32> = c.history.iter().copied().collect();
        if h.len() >= 2 {
            let (lo, hi) = h.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
            let span = (hi - lo).max(hi.abs() * 0.02).max(1.0);
            let mid = 0.5 * (hi + lo);
            let pt = |i: usize, v: f32| vec2(g.x + 4.0 + (g.w - 8.0) * i as f32 / (96 - 1) as f32, g.y + g.h * 0.6 - (v - mid) / span * (g.h * 0.7));
            for i in 1..h.len() {
                let (a, b) = (pt(i - 1, h[i - 1]), pt(i, h[i]));
                draw_line(a.x, a.y, b.x, b.y, 2.0, GREEN);
            }
        }
        let dev = c.sky_dev;
        let sky = if dev.abs() < 0.5 { "Ciel : comme d'habitude".to_string() } else { format!("Ciel : {:+.1} °C par rapport à d'habitude", dev) };
        ui.text(&sky, x, gy + g.h + 18.0, 13, TEXT);

        // The settings.
        let sy = gy + g.h + 34.0;
        ui.text("Réglages", x, sy + 6.0, 16, ACCENT);
        let mut vals = [c.sun_gain, c.space_gain, c.greenhouse];
        let rows: [(&str, f32, f32, Color); 3] = [("Soleil", 0.0, 2.0, ACCENT), ("Sortie vers l'espace", 0.5, 1.5, BLUE), ("Nuages et fumée gardent", 0.0, 1.0, GREEN)];
        for (k, (label, lo, hi, col)) in rows.iter().enumerate() {
            let y = sy + 16.0 + k as f32 * 24.0;
            ui.text(label, x, y + 13.0, 12, TEXT);
            ui.slider(Rect::new(x + 150.0, y, w - 150.0 - 60.0, 18.0), &mut vals[k], *lo, *hi, *col);
            ui.text(&format!("{:.0} %", vals[k] * 100.0), x + w - 40.0, y + 13.0, 12, DIM);
        }
        let c = &mut self.world.climate;
        (c.sun_gain, c.space_gain, c.greenhouse) = (vals[0], vals[1], vals[2]);
        let neutral = (c.sun_gain - 1.0).abs() < 0.01 && (c.space_gain - 1.0).abs() < 0.01 && (c.greenhouse - 1.0).abs() < 0.01;
        if ui.button(Rect::new(x, r.y + r.h - 36.0, w, 26.0), "Revenir au neutre", Icon::None, neutral, !neutral, GREEN) {
            c.reset_settings();
            self.say("Le soleil et le ciel reprennent leur cycle habituel".to_string());
        }
    }

    /// The time, the season and the wind, beside the energy budget: the Monde tab sets the starting
    /// point, these let the child play with them while the simulation runs.
    fn draw_live_weather(&mut self, ui: &mut Ui, r: Rect) {
        let (x, w) = (r.x + 4.0, r.w - 18.0);
        let half = (w - 6.0) / 2.0;
        ui.text("Temps", x, r.y + 24.0, 16, ACCENT);
        if ui.button(Rect::new(x, r.y + 32.0, half, 26.0), "Midi", Icon::Sun, false, true, ACCENT) {
            self.world.set_phase(0.25);
        }
        if ui.button(Rect::new(x + half + 6.0, r.y + 32.0, half, 26.0), "Minuit", Icon::Moon, false, true, BLUE) {
            self.world.set_phase(0.75);
        }
        let cycle = self.world.day_cycle;
        if ui.button(Rect::new(x, r.y + 62.0, w, 24.0), if cycle { "Jour et nuit : en marche" } else { "Jour et nuit : figé" }, Icon::None, cycle, true, GREEN) {
            self.world.day_cycle = !cycle;
        }
        let sy = r.y + 112.0;
        ui.text("Saison", x, sy, 16, ACCENT);
        for (i, s) in Season::ALL.iter().enumerate() {
            let b = Rect::new(x + (i % 2) as f32 * (half + 6.0), sy + 8.0 + (i / 2) as f32 * 28.0, half, 24.0);
            if ui.button(b, s.name(), Icon::None, self.world.season() == *s, true, ACCENT) {
                self.world.set_season(*s);
            }
        }
        let vy = sy + 84.0;
        ui.text("Vent", x, vy, 16, ACCENT);
        let mut vals = [self.world.wind_max, self.world.wind_changes, self.world.windiness];
        let rows: [(&str, f32, f32, String); 3] = [
            ("Force max", 0.05, 1.0, format!("{:.0} km/h", vals[0] * 40.0)),
            ("Virages", 0.0, 3.0, if vals[1] < 0.05 { "jamais".to_string() } else { format!("x{:.1}", vals[1]) }),
            ("Calme-venteux", 0.0, 1.0, format!("{:.0} %", vals[2] * 100.0)),
        ];
        for (k, (label, lo, hi, shown)) in rows.iter().enumerate() {
            let y = vy + 8.0 + k as f32 * 24.0;
            ui.text(label, x, y + 13.0, 12, TEXT);
            ui.slider(Rect::new(x + 96.0, y, w - 96.0 - 66.0, 18.0), &mut vals[k], *lo, *hi, BLUE);
            ui.text(shown, x + w - 58.0, y + 13.0, 12, DIM);
        }
        (self.world.wind_max, self.world.wind_changes, self.world.windiness) = (vals[0], vals[1], vals[2]);
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
        ui.layer("launch");
        ui.claim("launch", bar);
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
                self.remote_menu.show(2.0);
                self.selected_motor = None;
            }
        }
    }

    fn draw_ui(&mut self, ui: &mut Ui, sw: f32, sh: f32) {
        self.hover_info = None;
        // Under the leave-the-game window nothing answers a click.
        ui.muted = self.quit_modal;
        ui.layer("panel");
        // The palette: it slides out when the mouse goes to the left edge, and tucks away again.
        let handle_w = 16.0;
        let edge = PANEL_W + self.panel_shift();
        let handle = Rect::new(edge.max(0.0), TOP_H, handle_w, sh - TOP_H);
        let hovering = !ui.stroke && ((ui.mouse.x < edge.max(handle_w) && ui.mouse.y > TOP_H) || handle.contains(ui.mouse));
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
        ui.claim("panel", if self.panel.visible() { Rect::new(0.0, TOP_H, edge.max(0.0) + handle_w, sh - TOP_H) } else { handle });
        self.draw_tools_strip(ui, sh);
        self.draw_corner_buttons(ui, sw);
        self.draw_launch_bar(ui, sw);
        self.draw_views(ui, sw, sh);
        if self.mode == ViewMode::Simulation {
            match self.simview.panel(ui, &self.world, self.running, sw, sh) {
                simview::Ask::Run(run) => self.set_running(run),
                simview::Ask::Steps(n) => self.step_by_step(n),
                simview::Ask::None => {}
            }
        }
        self.draw_remote_menu(ui, sw);
        self.draw_player(ui, sw, sh);
        if self.climate_menu {
            self.draw_climate_menu(ui);
        }
        if self.show_science {
            self.draw_science(ui, sw, sh);
        }
        self.fps.frame();
        self.fps.draw(ui, sw, sh);
        if self.quit_modal {
            ui.muted = false;
            ui.layer("quit");
            ui.claim("quit", Rect::new(0.0, 0.0, sw, sh));
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
        let widths = [118.0f32, 108.0, 86.0, 38.0, 46.0, 38.0];
        let gap = 6.0;
        let pad = 12.0;
        let total: f32 = widths.iter().sum::<f32>() + gap * (widths.len() as f32 - 1.0) + pad * 2.0;
        let centre = self.left() + (sw - self.left()) / 2.0;
        let rest = Rect::new(centre - total / 2.0, sh - 66.0, total, 50.0);
        let over = ui.hover(rest);
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
        ui.layer("player");
        ui.claim("player", bar);
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
        let place = |i: usize, x: &mut f32| -> Rect {
            let r = Rect::new(*x, y, widths[i], h);
            *x += widths[i] + gap;
            r
        };
        let r = place(0, &mut x);
        if ui.button(r, "Réessayer", Icon::Reset, false, !self.building(), ACCENT) {
            self.reset();
        }
        if ui.hover(r) {
            self.hover_info = Some(("Réessayer (R)".to_string(), "Remet le monde tel que tu l'avais construit avant de lancer la simulation, remet les compteurs à zéro et revient à la construction. Ce qui a été fait pendant le test ou le défi est oublié.".to_string()));
        }
        let r = place(1, &mut x);
        let (label, icon) = if self.running { ("Pause", Icon::Pause) } else if self.building() { ("Tester", Icon::Play) } else { ("Reprendre", Icon::Play) };
        if ui.button(r, label, icon, !self.running, true, GREEN) {
            let run = !self.running;
            self.set_running(run);
        }
        if ui.hover(r) {
            self.hover_info = Some(("Tester (Espace)".to_string(), "Lance la simulation pour voir ce qui se passe. Tu peux encore bricoler pendant le test, même en pause, mais rien de tout ça n'est gardé : Réessayer revient au monde construit.".to_string()));
        }
        let r = place(2, &mut x);
        if ui.button(r, "Défi", Icon::Flag, self.challenge, true, RED) && !self.challenge {
            self.start_challenge();
        }
        if ui.hover(r) {
            self.hover_info = Some(("Défi".to_string(), "Repart du monde construit, compteurs à zéro, et verrouille la construction : on lance les voitures, et le lance-flammes, l'éclair et les capteurs restent là pour éprouver la construction. Réessayer pour recommencer ou reprendre la construction.".to_string()));
        }
        let r = place(3, &mut x);
        if ui.button(r, "", Icon::Minus, false, self.speed > 1, ACCENT) {
            self.speed -= 1;
        }
        let r = place(4, &mut x);
        ui.text_center(&format!("x{}", self.speed), r.x + r.w / 2.0, r.y + h / 2.0 + 6.0, 17, TEXT);
        let r = place(5, &mut x);
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
        if ui.button(Rect::new(6.0, y, half, 32.0), "Réessayer", Icon::Reset, false, !self.building(), ACCENT) {
            self.reset();
        }
        let label = if self.undo.is_empty() { "Annuler".to_string() } else { format!("Annuler {}", self.undo.len()) };
        if ui.button(Rect::new(12.0 + half, y, half, 32.0), &label, Icon::Undo, false, !self.undo.is_empty(), BLUE) {
            self.undo();
        }
    }

    /// The tools, always in reach on the left edge of the picture.
    fn draw_tools_strip(&mut self, ui: &mut Ui, sh: f32) {
        let tools: [(Tool, &str, Icon, &str); 9] = [
            (Tool::Select, "Sélection", Icon::Select, "Clique un élément (bloc, tige, engrenage, câble, tuyau, rail, voiture...) : il est entouré. Glisse-le pour le déplacer ; pour une tige, un câble, un tuyau ou un rail, glisse une poignée au bout pour le retailler. Suppr l'efface. Échap désélectionne. Une voiture sélectionnée affiche ses réglages."),
            (Tool::Eraser, "Gomme", Icon::Eraser, "Efface la matière, les blocs et les câbles (clic droit aussi)."),
            (Tool::Tunnel, "Tunnel", Icon::Tunnel, "Creuse comme la gomme, mais pose une voûte de briques au-dessus du trou : le sable, la terre ou l'eau du plafond ne s'effondrent plus dedans. La roche, elle, tient toute seule et reste telle quelle ; le sol du tunnel n'est pas touché. Le pinceau règle la largeur."),
            (Tool::Dynamite, "Dynamite", Icon::Dynamite, "Clique : un bâton de dynamite tombe à travers l'air et l'eau et se pose sur la matière. Clique dans la roche, la terre ou un mur pour l'y planter : il saute de l'intérieur et pulvérise la roche autour, comme dans une carrière. Plusieurs bâtons au même endroit s'additionnent. Sa mèche brûle 3 s quand la simulation tourne. Des bâtons qui se touchent sautent ensemble, plus fort, et l'onde de choc fait partir aussitôt les bâtons voisins. La détonation dépasse 100 000 atm : assez pour changer le charbon de bois en diamant."),
            (Tool::Flame, "Lance-flammes", Icon::Flame, "Garde le clic enfoncé : un jet de flammes à 1500 °C. Le bois prend feu, la pierre rougit puis fond en lave, le fer fond, l'eau bout. Ce qui est lourd (pierre, eau) chauffe plus lentement. Le pinceau règle la taille du jet."),
            (Tool::Lightning, "Éclair", Icon::Bolt, "Clique : la foudre tombe du ciel (du nuage au-dessus s'il y en a un) jusqu'au pointeur. Lâchée en l'air, elle saute vers le conducteur le plus proche ; sous terre, elle frappe la surface au-dessus. Tout son trajet chauffe : l'air à des milliers de degrés, le bois, l'herbe et le charbon prennent feu. Son courant file dans le sol par le meilleur conducteur, fond le sable en tube de verre (fulgurite), fait bouillir l'eau et court dans le fer. Le tonnerre suit, d'autant plus tard que c'est loin. Les gros nuages froids font leurs propres orages : vue Électricité (E)."),
            (Tool::Loupe, "Loupe", Icon::Loupe, "Le pointeur est une loupe : une vue ronde en bas à droite montre la matière sous la croix."),
            (Tool::Sensor, "Capteur", Icon::Thermo, "Pose une sonde (jusqu'à 4) : thermomètre, baromètre et hygromètre. Ses courbes s'affichent en bas de l'image : l'eau qui chauffe puis reste à 100 °C, une chaudière qui monte en pression, l'air qui sèche à midi. Clique sur une sonde pour l'enlever."),
            (Tool::Shovel, "Pelle", Icon::Shovel, "Clique un tas (cendre, sable, eau, fer...) : la bulle se remplit. Déplace-la, elle reste pleine. Garde le clic enfoncé : plus tu attends, plus le morceau est gros (jusqu'à tout) ; relâche pour le verser. La pelle ne se remplit que si elle est vide. Clic droit : tout verser. Le pinceau règle sa taille."),
        ];
        let (bw, gap) = (38.0, 6.0);
        let total = tools.len() as f32 * (bw + gap) + gap;
        let pill = Rect::new((PANEL_W + self.panel_shift()).max(0.0) + 8.0 + 16.0, (sh - total) / 2.0, bw + 12.0, total);
        ui.layer("tools");
        ui.claim("tools", pill);
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
        ui.layer("corner");
        ui.claim("corner", Rect::new(sci.x - 4.0, 4.0, s * 2.0 + gap + 12.0, s + 12.0));
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
            let inf = palette_info(m);
            let col = mat_color(m);
            let many = matter.states.len() > 1;
            let arrow = Rect::new(r.x + r.w - 25.0, r.y + 4.0, 21.0, 18.0);
            let on_arrow = many && ui.hover(arrow);
            if ui.hover(r) && menu.is_none_or(|(_, mr)| !ui.hover(mr)) {
                let mut fact = inf.fact.to_string();
                if matter.states.contains(&Mat::Trunk) {
                    fact = format!("{fact} Petite flèche du bouton : une bûche ou un arbre entier.");
                } else if many {
                    let names: Vec<&str> = matter.states.iter().map(|s| info(*s).name).collect();
                    fact = format!("{fact} Ses états : {} (petite flèche du bouton). La chaleur et le froid la font passer de l'un à l'autre.", names.join(", "));
                }
                self.hover_info = Some((format!("{} · {}", inf.name, inf.state), fact));
            }
            let clicked = ui.tile(r, inf.name, self.tool == matter_tool(m), |s| {
                if m == Mat::Trunk {
                    draw_tree_icon(s);
                    return;
                }
                rrect(s, 7.0, col);
                rrect(Rect::new(s.x, s.y, s.w, s.h * 0.4), 7.0, Color::new(1.0, 1.0, 1.0, 0.18));
                if matches!(m, Mat::Lava | Mat::Molten | Mat::MoltenSteel) {
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
                    self.tool = matter_tool(m);
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
                let inf = palette_info(*st);
                let hot = ui.hover(cr);
                let chosen = self.matter_state[i] == k;
                rrect(cr, 7.0, if hot { TILE_HOT } else { TILE });
                if chosen {
                    ui::rrect_lines(cr, 7.0, 2.0, ACCENT);
                }
                let swatch = Rect::new(cr.x + 5.0, cr.y + 5.0, cr.w - 10.0, 12.0);
                if *st == Mat::Trunk {
                    draw_tree_icon(Rect::new(swatch.x + swatch.w / 2.0 - 9.0, swatch.y - 2.0, 18.0, 16.0));
                } else {
                    rrect(swatch, 4.0, mat_color(*st));
                }
                let size = ui.fit_size(inf.name, cr.w - 4.0).min(14);
                ui.text_center(inf.name, cr.x + cr.w / 2.0, cr.y + cr.h - 6.0, size, if chosen { ACCENT } else { TEXT });
                if hot {
                    self.hover_info = Some((format!("{} · {}", inf.name, inf.state), inf.fact.to_string()));
                    if ui.pressed {
                        self.matter_state[i] = k;
                        self.tool = matter_tool(*st);
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
            Tool::Beam(m, role) => {
                let len = self.member_from.as_ref().map_or(60.0, |f| drafted(f.1).min(MEMBER_MAX_LEN));
                (m.cost_from(), m.cost(len, role))
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
        // The deck / girder strip under a rod button is drawn over what is below it, which must
        // not take its clicks.
        let rods = BlockKind::PLACEABLE.len() + CableMat::ALL.len() + SpringKind::ALL.len();
        let menu = self.beam_menu.map(|k| {
            let r = Self::tile_rect(rods + k, 3, top, 54.0);
            (k, Rect::new(6.0, r.y + r.h + 2.0, PANEL_W - 12.0, 50.0))
        });
        let menu_hover = menu.is_some_and(|(_, mr)| ui.hover(mr));
        let pressed = ui.pressed;
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
        // Free beams: pinned at both ends, like a bridge-building game. The little arrow of each
        // button chooses deck (the cars ride it) or girder (structure the cars pass through).
        ui.pressed = pressed && !menu_hover;
        let mut arrow_hit = false;
        for (k, mat) in MemberMat::ALL.iter().enumerate() {
            let r = Self::tile_rect(i + k, 3, top, 54.0);
            let role = self.beam_role[k];
            let arrow = Rect::new(r.x + r.w - 25.0, r.y + 4.0, 21.0, 18.0);
            let on_arrow = ui.hover(arrow);
            let (units, from) = (mat.cost(60.0, role), mat.cost_from());
            if ui.hover(r) && !menu_hover {
                let what = match role {
                    MemberRole::Deck => "Tablier : les voitures roulent dessus.",
                    MemberRole::Girder => "Poutre : la structure qui porte, les voitures passent à travers (comme les arbres).",
                };
                self.hover_info = Some((
                    format!("{} {}", role.name(), mat.name().to_lowercase()),
                    format!(
                        "{what} Petite flèche du bouton : tablier ou poutre. Un clic fixe un bout (sol, bloc ou nœud ; sans attache elle tombe), tire, un second clic pour l'autre bout. Longueur max {:.0}. 60 de long coûte {units} de {}.",
                        MEMBER_MAX_LEN,
                        info(from[0]).name.to_lowercase()
                    ),
                ));
            }
            let label = format!("{} {}", role.name(), mat.name().to_lowercase());
            let short = !free && self.stock(from) < units;
            let clicked = ui.tile(r, &label, self.tool == Tool::Beam(*mat, role), |s| draw_rod_icon(s, *mat, role));
            let open = self.beam_menu == Some(k);
            rrect(arrow, 5.0, if open { ACCENT } else if on_arrow { Color::new(0.0, 0.0, 0.0, 0.7) } else { Color::new(0.0, 0.0, 0.0, 0.45) });
            let (c, h) = (vec2(arrow.x + arrow.w / 2.0, arrow.y + arrow.h / 2.0), 4.5);
            let fg = if open { BG } else { TEXT };
            if open {
                draw_triangle(vec2(c.x - h, c.y + h * 0.5), vec2(c.x + h, c.y + h * 0.5), vec2(c.x, c.y - h * 0.6), fg);
            } else {
                draw_triangle(vec2(c.x - h, c.y - h * 0.5), vec2(c.x + h, c.y - h * 0.5), vec2(c.x, c.y + h * 0.6), fg);
            }
            if clicked {
                if on_arrow {
                    arrow_hit = true;
                    self.beam_menu = if open { None } else { Some(k) };
                } else {
                    self.tool = Tool::Beam(*mat, role);
                }
            }
            if short {
                Self::mark_missing(r);
            }
        }

        // Cable elasticity (the strip of a rod button may lie over it).
        let rows_used = (i + MemberMat::ALL.len() + 2) / 3;
        let sy = top + rows_used as f32 * 60.0 + 2.0;
        ui.pressed = pressed && !menu_hover;
        ui.text("Élasticité câble", 10.0, sy + 11.0, 13, DIM);
        let mut soft = self.cable_soft;
        ui.slider(Rect::new(110.0, sy, PANEL_W - 150.0, 18.0), &mut soft, 0.4, 3.0, ACCENT);
        if !menu_hover {
            self.cable_soft = soft;
        }
        ui.text(&format!("x{:.1}", self.cable_soft), PANEL_W - 34.0, sy + 11.0, 13, TEXT);
        let y0 = sy + 26.0;
        self.unlimited_toggles(ui, y0);
        ui.pressed = pressed;

        // The deck / girder strip of the rod button whose arrow was clicked.
        if let Some((k, mr)) = menu {
            let mat = MemberMat::ALL[k];
            rrect(Rect::new(mr.x, mr.y + 2.0, mr.w, mr.h), 10.0, Color::new(0.0, 0.0, 0.0, 0.45));
            rrect(mr, 10.0, PANEL);
            ui::rrect_lines(mr, 10.0, 2.0, ACCENT);
            let n = MemberRole::ALL.len();
            let w = (mr.w - 6.0 * (n as f32 + 1.0)) / n as f32;
            for (j, role) in MemberRole::ALL.iter().enumerate() {
                let cr = Rect::new(mr.x + 6.0 + j as f32 * (w + 6.0), mr.y + 6.0, w, mr.h - 12.0);
                let hot = ui.hover(cr);
                let chosen = self.beam_role[k] == *role;
                rrect(cr, 7.0, if hot { TILE_HOT } else { TILE });
                if chosen {
                    ui::rrect_lines(cr, 7.0, 2.0, ACCENT);
                }
                draw_rod_icon(Rect::new(cr.x + cr.w / 2.0 - 30.0, cr.y + 3.0, 60.0, 20.0), mat, *role);
                ui.text_center(role.name(), cr.x + cr.w / 2.0, cr.y + cr.h - 6.0, 14, if chosen { ACCENT } else { TEXT });
                if hot {
                    self.hover_info = Some((
                        format!("{} {}", role.name(), mat.name().to_lowercase()),
                        match role {
                            MemberRole::Deck => "Le tablier : la chaussée du pont, épaisse, avec ses croisillons. Les voitures roulent dessus et pèsent sur lui.".into(),
                            MemberRole::Girder => "La poutre : la charpente qui tient le tablier (diagonales, poteaux, arcs). Elle porte les efforts, mais les voitures la traversent, comme les arbres.".into(),
                        },
                    ));
                    if ui.pressed {
                        self.beam_role[k] = *role;
                        self.tool = Tool::Beam(mat, *role);
                        self.beam_menu = None;
                    }
                }
            }
            // A click anywhere else closes the strip.
            if ui.pressed && !arrow_hit && !ui.hover(mr) {
                self.beam_menu = None;
            }
        }

        // Materials: a block uses up what it is made of from the screen.
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
        let ty = sy0 + 78.0;
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
        let sy = ty + 76.0;
        ui.text("Saison", 10.0, sy, 16, ACCENT);
        for (i, s) in Season::ALL.iter().enumerate() {
            let r = Rect::new(6.0 + (i % 2) as f32 * (half + 6.0), sy + 6.0 + (i / 2) as f32 * 32.0, half, 27.0);
            if ui.button(r, s.name(), Icon::None, self.world.season() == *s, true, ACCENT) {
                self.world.set_season(*s);
            }
        }
        let iy = sy + 6.0 + 2.0 * 32.0 + 10.0;
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
            let y = iy + 30.0 + k as f32 * 20.0;
            ui.text(label, 10.0, y + 12.0, 12, TEXT);
            ui.slider(Rect::new(96.0, y, PANEL_W - 96.0 - 74.0, 18.0), &mut vals[k], *lo, *hi, BLUE);
            ui.text(shown, PANEL_W - 64.0, y + 13.0, 12, DIM);
        }
        self.world.wind_max = vals[0];
        self.world.wind_changes = vals[1];
        self.world.windiness = vals[2];
        let cr = Rect::new(10.0, iy + 30.0 + 3.0 * 20.0 + 2.0, PANEL_W - 20.0, 24.0);
        if ui.checkbox(cr, "Air compressible (P V = n R T)", self.world.flow.compress, true) {
            self.world.flow.compress = !self.world.flow.compress;
        }
        // Two ways of computing, side by side as toggles (the panel has no room for two more rows):
        // the cells' turn on every core, about twice as fast when much is moving, but water spreads
        // and drains a little differently (see `world/turn.rs`); and the liquids as particles (one
        // cell each) or by their physics (see `world/liquid.rs`).
        let half = (PANEL_W - 18.0) / 2.0;
        let pr = Rect::new(6.0, cr.y + 27.0, half, 24.0);
        let fr = Rect::new(12.0 + half, cr.y + 27.0, half, 24.0);
        if ui.button(pr, "Tous les cœurs", Icon::None, self.world.parallel_cells, true, GREEN) {
            self.world.parallel_cells = !self.world.parallel_cells;
        }
        let label = if self.world.liquid_physics { "Liquides : physique" } else { "Liquides : particules" };
        if ui.button(fr, label, Icon::None, self.world.liquid_physics, true, BLUE) {
            self.world.set_liquid_physics(!self.world.liquid_physics);
        }
        if ui.hover(pr) {
            self.hover_info = Some((
                "Calcul sur tous les cœurs".to_string(),
                "Jusqu'à deux fois plus vite quand beaucoup bouge, mais l'eau s'écoule un peu autrement.".to_string(),
            ));
        }
        if ui.hover(fr) {
            self.hover_info = Some((
                "Liquides : particules ou physique".to_string(),
                "Particules : chaque case de liquide est une particule. Physique (prototype) : tous les liquides ont une vitesse et une pression en unités réelles, la lave épaisse et figée en refroidissant.".to_string(),
            ));
        }
    }

    /// The palette of views on the right edge: it slides out while the mouse is over it (or at the
    /// edge), tucks away when the mouse leaves, and stays out when pinned, like the left palette.
    fn draw_views(&mut self, ui: &mut Ui, sw: f32, sh: f32) {
        let y = TOP_H + 60.0;
        let h = 430.0;
        let tab = Rect::new(sw - VIEWS_TAB, y, VIEWS_TAB, 96.0);
        let px = sw - VIEWS_TAB - VIEWS_W * self.views.open;
        let panel = Rect::new(px, y, VIEWS_W, h);
        let near_edge = ui.mouse.x > sw - 8.0 && ui.mouse.y > TOP_H && ui.mouse.y < y + h;
        let hovering = !ui.stroke && (tab.contains(ui.mouse) || (self.views.open > 0.05 && panel.contains(ui.mouse)) || near_edge);
        self.views.update(hovering, get_frame_time());
        let px = sw - VIEWS_TAB - VIEWS_W * self.views.open;
        let panel = Rect::new(px, y, VIEWS_W, h);
        ui.layer("views");
        ui.claim("views", if self.views.visible() { tab.combine_with(panel) } else { tab });

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
        let modes: [(ViewMode, &str, Icon, Color); 8] = [
            (ViewMode::Normal, "Normale", Icon::Sun, ACCENT),
            (ViewMode::Thermal, "Température", Icon::Thermo, RED),
            (ViewMode::Stress, "Efforts", Icon::Gauge, ACCENT),
            (ViewMode::Wind, "Vent", Icon::Wind, BLUE),
            (ViewMode::Pressure, "Pression", Icon::Gauge, GREEN),
            (ViewMode::Humidity, "Hygrométrie", Icon::Drop, BLUE),
            (ViewMode::Electric, "Électricité", Icon::Bolt, ACCENT),
            (ViewMode::Simulation, "Simulation", Icon::Science, GREEN),
        ];
        for (i, (m, label, icon, col)) in modes.iter().enumerate() {
            let r = Rect::new(panel.x + 8.0, panel.y + 32.0 + i as f32 * 31.0, panel.w - 16.0, 27.0);
            if ui.button(r, label, *icon, self.mode == *m, true, *col) {
                self.mode = *m;
            }
        }
        // Legend for the chosen view.
        let ly = panel.y + 32.0 + modes.len() as f32 * 31.0 + 8.0;
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
                ui.wrapped("Chaque grain a sa température. Points clairs : grains de vapeur, nuage ou fumée portés par le vent.", bar.x, bar.y + 42.0, bar.w, 11, DIM);
            }
            ViewMode::Stress => {
                for i in 0..40 {
                    let (r2, g2, b2) = render::stress_rgb(i as f32 / 39.0);
                    draw_rectangle(bar.x + bar.w * i as f32 / 40.0, bar.y, bar.w / 40.0 + 1.0, bar.h, Color::new(r2, g2, b2, 1.0));
                }
                ui.text("détendu", bar.x, bar.y + 25.0, 11, DIM);
                ui.text("casse", bar.x + bar.w - 28.0, bar.y + 25.0, 11, DIM);
                ui.wrapped("Rouge = chargé. Solides : part de la résistance de la section qui porte le reste (au-delà, elle casse). Mécanique : anneau = dents, arc = couple, F = force, % = charge moteur.", bar.x, bar.y + 42.0, bar.w, 11, DIM);
            }
            ViewMode::Simulation => {
                ui.wrapped("Le panneau « Simulation » choisit le fond (matière, classes, pressions, diff du dernier pas) et les couches (zones, vitesses, gouttes, carte), et fait avancer le monde pas à pas (touche « . »).", bar.x, bar.y + 4.0, bar.w, 11, DIM);
            }
            ViewMode::Wind => {
                let w = &self.world;
                ui.text(&format!("{:.0} km/h {}", w.wind.abs() * 40.0, if w.wind > 0.0 { "→" } else { "←" }), bar.x, bar.y + 4.0, 14, TEXT);
                ui.wrapped("Couches de vent opposées et tourbillons : ils portent nuages, vapeur, fumée et pluie, et contournent le relief.", bar.x, bar.y + 22.0, bar.w, 11, DIM);
            }
            ViewMode::Pressure => {
                // Two scales, both logarithmic: the air (from the thin air at the top of the sky to a
                // boiler) and the ground and liquids (up to tens of atmospheres).
                let air_atm = |u: f32| world::AIR_LOW_ATM * (world::AIR_TOP_ATM / world::AIR_LOW_ATM).powf(u);
                let ground = Rect::new(bar.x, bar.y + 44.0, bar.w, bar.h);
                for i in 0..40 {
                    let u = i as f32 / 39.0;
                    let (r2, g2, b2) = world::air_pressure_rgb(air_atm(u));
                    draw_rectangle(bar.x + bar.w * i as f32 / 40.0, bar.y, bar.w / 40.0 + 1.0, bar.h, Color::from_rgba(r2, g2, b2, 255));
                    let (r2, g2, b2) = world::ground_pressure_rgb(world::GROUND_TOP_ATM.powf(u));
                    draw_rectangle(ground.x + ground.w * i as f32 / 40.0, ground.y, ground.w / 40.0 + 1.0, ground.h, Color::from_rgba(r2, g2, b2, 255));
                }
                ui.text("Air", bar.x, ly + 10.0, 11, TEXT);
                for (atm, label) in [(0.05f32, "0,05"), (0.2, "0,2"), (1.0, "1"), (5.0, "5")] {
                    let u = world::air_scale(atm);
                    ui.text(label, bar.x + bar.w * u - 4.0, bar.y + 25.0, 11, DIM);
                }
                ui.text("atm", bar.x + bar.w - 20.0, ly + 10.0, 11, DIM);
                ui.text("Sol et liquides", ground.x, ground.y - 4.0, 11, TEXT);
                for (atm, label) in [(1.0f32, "1"), (3.0, "3"), (10.0, "10"), (30.0, "30")] {
                    let u = world::ground_scale(atm);
                    ui.text(label, ground.x + (ground.w * u - 4.0).min(ground.w - 12.0), ground.y + 25.0, 11, DIM);
                }
                ui.wrapped("Air : plus mince en altitude, plus fort dans une poche fermée. Sol et liquides : le poids de ce qui est au-dessus. Traits sombres : frontières gaz / liquide / solide.", bar.x, ground.y + 42.0, bar.w, 11, DIM);
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
                ui.wrapped("Air : part de vapeur d'eau par rapport à ce que l'air peut tenir (l'air froid en tient peu). Liquides : toujours 100 %. Points clairs : vapeur et nuage, de l'air saturé (100 %). Gris : solide sec ; la terre garde son humidité.", bar.x, bar.y + 42.0, bar.w, 11, DIM);
            }
            ViewMode::Electric => {
                for i in 0..40 {
                    let q = i as f32 / 39.0 * 2.0 - 1.0;
                    let a = q.abs().sqrt() * 0.85;
                    let t = if q > 0.0 { (255.0, 70.0, 60.0) } else { (70.0, 130.0, 255.0) };
                    let c = |b: f32, t: f32| ((b + (t - b) * a) / 255.0).min(1.0);
                    draw_rectangle(bar.x + bar.w * i as f32 / 40.0, bar.y, bar.w / 40.0 + 1.0, bar.h, Color::new(c(26.0, t.0), c(26.0, t.1), c(32.0, t.2), 1.0));
                }
                ui.text("−", bar.x, bar.y + 25.0, 11, DIM);
                ui.text("0", bar.x + bar.w * 0.5 - 3.0, bar.y + 25.0, 11, DIM);
                ui.text("+", bar.x + bar.w - 8.0, bar.y + 25.0, 11, DIM);
                let peak = (self.world.charge.iter().fold(0.0f32, |a, q| a.max(q.abs())) / world::BREAKDOWN)
                    .max(self.world.mcharge.iter().fold(0.0f32, |a, q| a.max(q.abs())) / world::GROUND_BREAKDOWN);
                ui.text(&format!("Claquage : {:.0} %", peak.min(1.0) * 100.0), bar.x, bar.y + 44.0, 13, TEXT);
                ui.wrapped("Dans un gros nuage froid et ascendant, les cristaux de glace (+) montent en haut, le grésil (−) reste en bas. Sous le nuage, le sol et ce qui s'y dresse prennent la charge opposée, surtout ce qui est haut et conduit. Quand la tension atteint le claquage, l'éclair part, et il laisse sa charge là où il frappe. Or : métal, cyan : eau.", bar.x, bar.y + 60.0, bar.w, 11, DIM);
            }
            ViewMode::Normal => {
                ui.wrapped("Touches : H température, X efforts, V vent, P pression, Y hygrométrie, E électricité.", bar.x, bar.y + 2.0, bar.w, 11, DIM);
            }
        }
        let _ = sh;
    }

    /// The motor's remote control, or the car settings: a side menu of its own under the views,
    /// whose tab shows while a motor or a car is chosen. It peeks out when one is chosen, then
    /// behaves like the other side menus: out on hover, away when the mouse leaves, kept by its pin.
    fn draw_remote_menu(&mut self, ui: &mut Ui, sw: f32) {
        let y = TOP_H + 60.0 + 376.0 + 8.0;
        let rest = Rect::new(sw - VIEWS_TAB - VIEWS_W, y, VIEWS_W, 0.0);
        let h = match self.remote_rect(rest) {
            Some(r) => r.h,
            None if self.car_remote => 286.0,
            None => {
                // Nothing to show: the next motor or car chosen peeks out again.
                self.remote_menu.open = 0.0;
                self.remote_menu.peek = 0.0;
                return;
            }
        };
        let motor = !self.car_remote || self.remote_rect(rest).is_some();
        let tab = Rect::new(sw - VIEWS_TAB, y, VIEWS_TAB, 96.0);
        let panel = Rect::new(sw - VIEWS_TAB - VIEWS_W * self.remote_menu.open, y, VIEWS_W, h);
        let near_edge = ui.mouse.x > sw - 8.0 && ui.mouse.y >= y && ui.mouse.y < y + h;
        let hovering = !ui.stroke && (tab.contains(ui.mouse) || (self.remote_menu.open > 0.05 && panel.contains(ui.mouse)) || near_edge);
        self.remote_menu.update(hovering, get_frame_time());
        let panel = Rect::new(sw - VIEWS_TAB - VIEWS_W * self.remote_menu.open, y, VIEWS_W, h);
        ui.layer("remote");
        ui.claim("remote", if self.remote_menu.visible() { tab.combine_with(panel) } else { tab });

        // The tab that stays visible while something is chosen: click it to pin the menu.
        rrect(tab, 8.0, Color::new(0.1, 0.12, 0.2, 0.92));
        let c = vec2(tab.x + tab.w / 2.0, tab.y + 22.0);
        if motor {
            draw_icon(Icon::Rotate, c, 15.0, ACCENT);
        } else {
            draw_car_icon(c, ACCENT, 0.45);
        }
        let arrow = if self.remote_menu.pin { "›" } else { "‹" };
        ui.text_center(arrow, tab.x + tab.w / 2.0, tab.y + 56.0, 20, if self.remote_menu.pin { ACCENT } else { TEXT });
        if ui.pressed && tab.contains(ui.mouse) && !self.quit_modal {
            self.remote_menu.toggle_pin();
        }
        if !self.remote_menu.visible() {
            return;
        }
        if motor {
            self.draw_remote(ui, panel);
        } else {
            self.draw_car_remote(ui, panel);
        }
        // The car settings have their × at the top right: the pin goes beside it.
        let pin = Rect::new(panel.x + panel.w - if motor { 34.0 } else { 64.0 }, panel.y + 5.0, 28.0, 24.0);
        if ui.button(pin, "", Icon::Pin, self.remote_menu.pin, true, ACCENT) {
            self.remote_menu.toggle_pin();
        }
        if ui.hover(pin) {
            let what = if motor { "la télécommande" } else { "les réglages" };
            self.hover_info = Some(("Épingler".to_string(), format!("Épinglé, {what} reste ouvert. Sinon il se cache quand la souris s'en va et ressort au survol de son onglet.")));
        }
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
                ui.text("Espace : tester / pause", r.x + 8.0, r.y + 19.0, 13, DIM);
                ui.text("R réessayer · C voiture · Z annuler", r.x + 8.0, r.y + 36.0, 13, DIM);
                ui.text("H thermique · X efforts · V vent", r.x + 8.0, r.y + 53.0, 13, DIM);
                ui.text("Ctrl + molette/glisser : zoom", r.x + 8.0, r.y + 70.0, 13, DIM);
                ui.text("Survole une case pour apprendre", r.x + 8.0, r.y + 90.0, 13, DIM);
            }
        }
    }

    fn draw_science(&mut self, ui: &mut Ui, sw: f32, sh: f32) {
        draw_rectangle(0.0, TOP_H, sw, sh - TOP_H, Color::new(0.02, 0.03, 0.07, 0.86));
        ui.layer("science");
        ui.claim("science", Rect::new(0.0, TOP_H, sw, sh - TOP_H));
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
        if max_row > 0 && !ui.stroke && is_mouse_button_down(MouseButton::Left) && Rect::new(track.x - 8.0, track.y, track.w + 16.0, track.h).contains(ui.mouse) {
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
/// `--tunnel`: two sand dunes on bedrock, both dug through. The left one with the eraser (the sand
/// pours back in), the right one with the tunnel tool (the vault holds), then the world runs.
/// Liquids under pressure: two vessels of water joined at the bottom, a siphon draining a tank
/// over a crest into a basin, and a U-tube of oil.
/// Trees planted from the palette, and logs painted in the air: each falls as one piece and they
/// pile up instead of melting into one another or into the ground.
fn demo_logs(g: &mut Game) {
    g.empty_world();
    g.blocks.clear();
    g.cables.clear();
    g.members.clear();
    let ground = 430;
    for x in 0..W as i32 {
        for y in ground..H as i32 {
            g.world.set(x, y, if y < ground + 3 { Mat::Grass } else if y < ground + 20 { Mat::Dirt } else { Mat::Stone });
        }
    }
    // A rock wall: a log painted against its side does not stick to it.
    for x in 640..700 {
        for y in 330..ground {
            g.world.set(x, y, Mat::Stone);
        }
    }
    for x in [140, 330, 860] {
        g.world.plant_tree(x, 0, 0.6);
    }
    for (k, (x, len)) in [(500, 80), (512, 56), (494, 64), (506, 40)].into_iter().enumerate() {
        g.world.new_log();
        let y = ground - 60 - k as i32 * 26;
        g.world.line((x - len / 2, y), (x + len / 2, y), 3, Mat::Wood);
    }
    g.world.new_log();
    g.world.line((703, 300), (740, 300), 3, Mat::Wood);
    // A standing log, painted upright.
    g.world.new_log();
    g.world.line((580, ground - 120), (580, ground - 80), 3, Mat::Wood);
}

fn demo_siphon(g: &mut Game) {
    g.empty_world();
    let rect = |g: &mut Game, x0: i32, y0: i32, x1: i32, y1: i32, m: Mat| {
        for y in y0..=y1 {
            for x in x0..=x1 {
                g.world.set(x, y, m);
            }
        }
    };
    rect(g, 0, 470, W as i32 - 1, H as i32 - 1, Mat::Stone);
    // Two vessels, the left one full.
    rect(g, 60, 300, 260, 469, Mat::Stone);
    rect(g, 70, 300, 120, 460, Mat::Empty);
    rect(g, 200, 300, 250, 460, Mat::Empty);
    rect(g, 121, 452, 199, 460, Mat::Empty);
    rect(g, 70, 340, 120, 460, Mat::Water);
    // A siphon: an open tank on a pedestal, a full pipe over its rim down into a basin below.
    rect(g, 330, 250, 480, 360, Mat::Stone);
    rect(g, 340, 250, 440, 350, Mat::Empty);
    rect(g, 340, 275, 440, 350, Mat::Water);
    rect(g, 330, 361, 345, 469, Mat::Stone);
    rect(g, 425, 238, 425, 300, Mat::Stone);
    rect(g, 430, 244, 430, 300, Mat::Stone);
    rect(g, 425, 239, 530, 239, Mat::Stone);
    rect(g, 431, 244, 525, 244, Mat::Stone);
    rect(g, 525, 244, 525, 440, Mat::Stone);
    rect(g, 530, 239, 530, 440, Mat::Stone);
    rect(g, 482, 245, 486, 469, Mat::Stone);
    rect(g, 426, 240, 429, 300, Mat::Water);
    rect(g, 426, 240, 529, 243, Mat::Water);
    rect(g, 526, 244, 529, 440, Mat::Water);
    rect(g, 490, 410, 490, 469, Mat::Stone);
    rect(g, 600, 410, 600, 469, Mat::Stone);
    // A U-tube of oil, filled on the left.
    rect(g, 680, 300, 880, 469, Mat::Stone);
    rect(g, 690, 300, 730, 460, Mat::Empty);
    rect(g, 830, 300, 870, 460, Mat::Empty);
    rect(g, 731, 452, 829, 460, Mat::Empty);
    rect(g, 690, 360, 730, 460, Mat::Oil);
    // The thin walls between the vessels and around the pipes hang free: keep them up.
    g.world.structures_fall = false;
    g.set_running(true);
}

fn demo_tunnel(g: &mut Game) {
    g.empty_world();
    let floor = 420;
    for x in 0..W as i32 {
        for y in floor..H as i32 {
            g.world.set(x, y, Mat::Stone);
        }
    }
    for (x0, x1) in [(40i32, 440i32), (520, 920)] {
        let (mid, half) = ((x0 + x1) / 2, (x1 - x0) / 2);
        for x in x0..x1 {
            let h = 30 + 80 * (half - (x - mid).abs()) / half;
            for y in floor - h..floor {
                g.world.set(x, y, Mat::Sand);
            }
        }
    }
    let y = floor - 16;
    g.world.line((20, y), (460, y), 9, Mat::Empty);
    g.world.tunnel_line((500, y), (940, y), 9);
    g.tool = Tool::Tunnel;
    g.set_running(true);
}

/// `--dam`: two dams on bedrock, liquids by their physics. On the left a thick wall of rock holds
/// back a deep lake; on the right a pane of glass holds a shallower one, until the water pushes
/// it over. Best seen in the stress view (`--view efforts`).
fn demo_dam(g: &mut Game) {
    g.empty_world();
    g.world.set_liquid_physics(true);
    let floor = 440;
    for x in 0..W as i32 {
        for y in floor..H as i32 {
            g.world.set(x, y, Mat::Stone);
        }
    }
    let mut rect = |x0: i32, y0: i32, x1: i32, y1: i32, m: Mat| {
        for y in y0..=y1 {
            for x in x0..=x1 {
                g.world.set(x, y, m);
            }
        }
    };
    rect(56, 370, 59, floor - 1, Mat::Stone);
    rect(300, 380, 303, floor - 1, Mat::Stone);
    rect(60, 390, 299, floor - 1, Mat::Water);
    rect(514, 370, 519, floor - 1, Mat::Stone);
    rect(700, 380, 700, floor - 1, Mat::Glass);
    rect(520, 390, 699, floor - 1, Mat::Water);
    for i in 0..W * H {
        g.world.temp[i] = 15.0;
    }
    g.set_running(true);
}

/// A few bricks on flat rock, the brick tool in hand: with `--mouse` near them, the ghost is pulled
/// into line and the guides show (`--magnet`).
fn demo_magnet(g: &mut Game) {
    g.empty_world();
    let floor = 420;
    for x in 0..W as i32 {
        for y in floor..H as i32 {
            g.world.set(x, y, Mat::Stone);
        }
    }
    let f = floor as f32;
    for (k, x, y) in [(BlockKind::Long, 400.0, f - 5.0), (BlockKind::Brick, 390.0, f - 15.0), (BlockKind::Post, 470.0, f - 20.0), (BlockKind::Cube, 540.0, f - 10.0)] {
        g.blocks.push(Block::new(k, vec2(x, y), 0.0));
    }
    g.refresh_mask();
    g.tool = Tool::Block(BlockKind::Brick);
}

/// `--sticks`: sticks of dynamite on a meadow, their fuses burning: a heap of three on the grass,
/// one planted in the ground, one sinking in a pond (`--frames` before 180 to see them burn).
fn demo_sticks(g: &mut Game) {
    g.empty_world();
    for x in 0..W as i32 {
        g.world.set(x, 330, Mat::Grass);
        for y in 331..H as i32 {
            g.world.set(x, y, if y > 430 { Mat::Stone } else { Mat::Dirt });
        }
    }
    for x in 600..700 {
        for y in 330..356 {
            g.world.set(x, y, Mat::Water);
        }
    }
    for _ in 0..3 {
        g.dynamite.drop(&g.world, vec2(300.0, 200.0));
    }
    g.dynamite.drop(&g.world, vec2(480.0, 360.0));
    g.dynamite.settle(&g.world);
    g.dynamite.drop(&g.world, vec2(650.0, 300.0));
    g.tool = Tool::Dynamite;
    g.set_running(true);
}

/// `--quarry`: sticks planted in rock: one alone, four in the same blast hole, and a row of three
/// a little apart, which the shock of the first sets off one after the other.
fn demo_quarry(g: &mut Game) {
    g.empty_world();
    for x in 0..W as i32 {
        for y in 300..H as i32 {
            g.world.set(x, y, Mat::Stone);
        }
    }
    g.dynamite.drop(&g.world, vec2(180.0, 360.0));
    for _ in 0..4 {
        g.dynamite.drop(&g.world, vec2(470.0, 380.0));
    }
    for (k, x) in [720.0, 745.0, 770.0].into_iter().enumerate() {
        g.dynamite.drop(&g.world, vec2(x, 350.0));
        g.dynamite.sticks.last_mut().unwrap().fuse = dynamite::FUSE + 60 * k as u32;
    }
    g.tool = Tool::Dynamite;
    g.set_running(true);
}

/// A meadow over rock with a big charge of powder buried in it: `--crater` lights it. With
/// `dynamite`, the charge is a stick of dynamite packed in charcoal (it leaves diamonds).
fn demo_crater(g: &mut Game, dynamite: bool) {
    g.empty_world();
    let ground = 330;
    for x in 0..W as i32 {
        let top = ground + ((x as f32 * 0.02).sin() * 4.0) as i32;
        g.world.set(x, top, Mat::Grass);
        for y in top + 1..H as i32 {
            g.world.set(x, y, if y > 430 + (x * 7 % 5) { Mat::Stone } else { Mat::Dirt });
        }
    }
    // 40 x 24 cells of powder, its top 30 cells under the grass.
    for x in 460..500 {
        for y in 360..384 {
            let core = (472..488).contains(&x) && (366..378).contains(&y);
            g.world.set(x, y, if !dynamite { Mat::Gunpowder } else if core { Mat::Dynamite } else { Mat::Charcoal });
        }
    }
    g.set_running(true);
}

/// `--demo --lava`: underground pockets of lava, a big one and a small one, to see the light they
/// cast at night, and a heap of charcoal and an iron post on the surface (for `--lightning`).
fn demo_lava(g: &mut Game) {
    g.empty_world();
    for x in 0..W as i32 {
        g.world.set(x, 330, Mat::Grass);
        for y in 331..H as i32 {
            g.world.set(x, y, Mat::Stone);
        }
    }
    for x in 700..740 {
        for y in 330 - (20 - (x - 720i32).abs() / 2)..330 {
            g.world.set(x, y, Mat::Charcoal);
        }
    }
    for y in 290..330 {
        g.world.set(800, y, Mat::Iron);
    }
    for (cx, cy, r) in [(400i32, 380i32, 30i32), (620, 390, 3)] {
        for x in cx - r..=cx + r {
            for y in cy - r..=cy + r {
                if (x - cx).pow(2) + (y - cy).pow(2) <= r * r {
                    g.world.set(x, y, Mat::Lava);
                }
            }
        }
    }
}

/// `--demo --topple`: a long stroke of ice painted from the ground up into the sky, leaning far
/// out, as a player draws it.
fn demo_topple(g: &mut Game) {
    g.empty_world();
    // Fields of earth over the rock, and a pond in a hollow.
    for x in 0..W as i32 {
        let top = 330 + ((x as f32 * 0.013).sin() * 8.0) as i32;
        let bed = if (380..540).contains(&x) { top + (40.0 * (std::f32::consts::PI * (x - 380) as f32 / 160.0).sin()) as i32 } else { top };
        for y in top..H as i32 {
            let m = if y > 380 + (x * 7 % 5) { Mat::Stone } else if y < bed { Mat::Water } else if y == top { Mat::Grass } else { Mat::Dirt };
            g.world.set(x, y, m);
        }
    }
    let ground = |w: &World, x: i32| (0..H as i32).find(|&y| w.at(x, y).is_hard()).unwrap_or(H as i32);
    let foot = (440, ground(&g.world, 440) - 2);
    let stroke: [(i32, i32); 5] = [foot, (520, 140), (545, 122), (600, 150), (720, 134)];
    for seg in stroke.windows(2) {
        let (a, b) = (seg[0], seg[1]);
        let n = (b.0 - a.0).abs().max((b.1 - a.1).abs()).max(1);
        for k in 0..=n {
            g.world.paint(a.0 + (b.0 - a.0) * k / n, a.1 + (b.1 - a.1) * k / n, 4, Mat::Ice);
        }
    }
    g.set_running(true);
}

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
    let click = |g: &mut Game, tool: Tool, x: f32, y: f32| {
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
/// The tool a state of the palette picks: the trunk stands for a whole tree.
fn matter_tool(m: Mat) -> Tool {
    if m == Mat::Trunk {
        Tool::Tree
    } else {
        Tool::Mat(m)
    }
}

/// A little tree on a strip of grass, for the palette.
fn draw_tree_icon(s: Rect) {
    rrect(s, 7.0, Color::from_rgba(120, 170, 220, 255));
    rrect(Rect::new(s.x, s.y + s.h * 0.8, s.w, s.h * 0.2), 7.0, Color::from_rgba(88, 172, 64, 255));
    let cx = s.x + s.w * 0.5;
    let trunk = mat_color(Mat::Trunk);
    draw_rectangle(cx - s.w * 0.05, s.y + s.h * 0.4, s.w * 0.1, s.h * 0.42, trunk);
    draw_circle(cx, s.y + s.h * 0.36, s.h * 0.27, Color::from_rgba(52, 140, 60, 255));
    draw_circle(cx, s.y + s.h * 0.38, s.h * 0.08, mat_color(Mat::Branch));
}

/// A rod for the palette: a deck lies flat, thick, with cross bracing; a girder is a plain bar
/// on the slant.
fn draw_rod_icon(s: Rect, mat: MemberMat, role: MemberRole) {
    let (cr, cg, cb) = mat.color();
    let col = Color::from_rgba(cr, cg, cb, 255);
    let pin = |p: Vec2| {
        draw_circle(p.x, p.y, 4.5, Color::from_rgba(226, 228, 236, 255));
        draw_circle(p.x, p.y, 2.0, Color::from_rgba(60, 64, 78, 255));
    };
    match role {
        MemberRole::Deck => {
            let (x0, x1, y) = (s.x + 6.0, s.x + s.w - 6.0, s.y + s.h * 0.5);
            let h = (s.h * 0.5).clamp(6.0, 12.0);
            draw_rectangle(x0, y - h / 2.0, x1 - x0, h, Color::new(col.r * 0.55, col.g * 0.55, col.b * 0.55, 1.0));
            draw_line(x0, y - h / 2.0 + 1.0, x1, y - h / 2.0 + 1.0, 2.0, col);
            draw_line(x0, y + h / 2.0 - 1.0, x1, y + h / 2.0 - 1.0, 2.0, col);
            let brace = Color::new(col.r * 0.95, col.g * 0.95, col.b * 0.95, 0.9);
            let n = (((x1 - x0) / h).round() as i32).max(2);
            let w = (x1 - x0) / n as f32;
            for k in 0..n {
                let xa = x0 + k as f32 * w;
                draw_line(xa, y - h / 2.0 + 1.0, xa + w, y + h / 2.0 - 1.0, 1.0, brace);
                draw_line(xa, y + h / 2.0 - 1.0, xa + w, y - h / 2.0 + 1.0, 1.0, brace);
            }
            draw_line(x0, y - h / 2.0, x1, y - h / 2.0, 1.2, Color::new(1.0, 0.95, 0.6, 0.9));
            pin(vec2(x0, y));
            pin(vec2(x1, y));
        }
        MemberRole::Girder => {
            let (a, b) = (vec2(s.x + 10.0, s.y + s.h - 6.0), vec2(s.x + s.w - 10.0, s.y + 6.0));
            draw_line(a.x, a.y, b.x, b.y, 5.0, col);
            pin(a);
            pin(b);
        }
    }
}

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
    let mut ui = Ui::new(font);
    let mut renderer = Renderer::new();
    let mut game = Game::new();
    #[cfg(feature = "sound")]
    if shot.is_none() && !args.iter().any(|a| a == "--mute") {
        game.sounds = Sounds::load().await;
    }
    if shot.is_some() {
        // Screenshots stay as they were: no frame counter in them.
        game.fps.shown = args.iter().any(|a| a == "--fps");
        game.metrics.shown = args.iter().any(|a| a == "--metrics");
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
        // `--sow`: seeds sown along the ground, all across the world (before `--lawn`'s steps).
        if args.iter().any(|a| a == "--sow") {
            for x in (0..W as i32).step_by(7) {
                let y = (0..H as i32).find(|&y| matches!(game.world.raw(x, y), Mat::Dirt | Mat::Grass)).unwrap_or(0);
                game.world.paint(x, y + 2, 2, Mat::Seed);
            }
        }
        // `--lawn n`: wet earth on the left, dry on the right, and n steps for the grass to answer.
        if let Some(n) = arg("--lawn").and_then(|v| v.parse::<u32>().ok()) {
            game.world.demo_lawn(n);
        }
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
        // `--day n`: that day of the moon's cycle (3 is a full moon), at the same hour.
        if let Some(d) = arg("--day").and_then(|d| d.parse::<u32>().ok()) {
            game.world.set_day(d);
        }
        game.mode = match arg("--view").as_deref() {
            Some("thermique") => ViewMode::Thermal,
            Some("efforts") => ViewMode::Stress,
            Some("vent") => ViewMode::Wind,
            Some("pression") => ViewMode::Pressure,
            Some("hygro") => ViewMode::Humidity,
            Some("electricite") => ViewMode::Electric,
            Some("simulation") => ViewMode::Simulation,
            _ => ViewMode::Normal,
        };
        // `--sim-back`: the simulation view's background.
        if let Some(b) = arg("--sim-back") {
            use simview::{Back, DiffOf};
            game.simview.back = match b.as_str() {
                "matiere" => Back::Matter,
                "pression" => Back::Pressure,
                "air" => Back::Air,
                "calcul" => Back::Classes,
                "diff-remplissage" => Back::Diff(DiffOf::Fill),
                "diff-temperature" => Back::Diff(DiffOf::Temp),
                "diff-pression" => Back::Diff(DiffOf::Pressure),
                "diff-vitesse" => Back::Diff(DiffOf::Velocity),
                _ => Back::Matter,
            };
            game.simview.velocity[3] = args.iter().any(|a| a == "--sim-air");
        }
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
                game.remote_menu = Drawer::new(true);
            }
        }
        game.show_science = args.iter().any(|a| a == "--science");
        game.climate_menu = args.iter().any(|a| a == "--climate");
        game.player_pin = args.iter().any(|a| a == "--player");
        game.tool = if args.iter().any(|a| a == "--loupe") { Tool::Loupe } else { Tool::Block(BlockKind::Brick) };
        // `--states N` opens the states of the palette's matter N (1 is water).
        if let Some(i) = arg("--states").and_then(|v| v.parse::<usize>().ok()).filter(|&i| i < MATTERS.len()) {
            game.tab = Tab::Matter;
            game.matter_menu = Some(i);
            game.tool = matter_tool(MATTERS[i].states[MATTERS[i].default]);
        }
        // `--rods N` opens the deck / girder strip of the palette's rod N (0 is wood).
        if let Some(k) = arg("--rods").and_then(|v| v.parse::<usize>().ok()).filter(|&k| k < MemberMat::ALL.len()) {
            game.tab = Tab::Blocks;
            game.beam_menu = Some(k);
            game.tool = Tool::Beam(MemberMat::ALL[k], game.beam_role[k]);
        }
        if args.iter().any(|a| a == "--tunnel") {
            demo_tunnel(&mut game);
        }
        if args.iter().any(|a| a == "--dam") {
            demo_dam(&mut game);
        }
        if args.iter().any(|a| a == "--magnet") {
            demo_magnet(&mut game);
        }
        if args.iter().any(|a| a == "--crater") {
            demo_crater(&mut game, args.iter().any(|a| a == "--dynamite"));
        }
        if args.iter().any(|a| a == "--sticks") {
            demo_sticks(&mut game);
        }
        if args.iter().any(|a| a == "--quarry") {
            demo_quarry(&mut game);
        }
        if args.iter().any(|a| a == "--topple") {
            demo_topple(&mut game);
        }
        if args.iter().any(|a| a == "--siphon") {
            demo_siphon(&mut game);
        }
        if args.iter().any(|a| a == "--logs") {
            demo_logs(&mut game);
        }
        if args.iter().any(|a| a == "--lava") {
            demo_lava(&mut game);
            // A bolt is only drawn while the world runs.
            game.set_running(arg("--lightning").is_some());
        }
        // `--advance n`: the world runs n steps before the first frame.
        for _ in 0..arg("--advance").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0) {
            game.world.step();
        }
        // `--challenge`: the challenge is on (the HUD and the player show it).
        if args.iter().any(|a| a == "--challenge") {
            game.start_challenge();
        }
    }
    // `--mouse x,y` pins the pointer for screenshots.
    let demo_mouse = arg("--mouse").and_then(|v| {
        let (x, y) = v.split_once(',')?;
        Some(vec2(x.parse().ok()?, y.parse().ok()?))
    });
    let mut frame = 0u32;
    let mut frame_time = 0.0f32;
    // The drawing of the world and of the interface, in milliseconds (smoothed): for the metrics panel.
    let mut draw_ms = (0.0f32, 0.0f32);
    // The world runs in its own thread on the desktop (see `simthread.rs`). Not on the web (no
    // threads there), nor for the screenshots, which count their frames (unless `--threads`: the
    // shot is then taken as the screen sees it); `--one-thread` keeps the one loop, to compare.
    let threads = if shot.is_some() { args.iter().any(|a| a == "--threads") } else { !args.iter().any(|a| a == "--one-thread") };
    let mut sim = (cfg!(not(target_arch = "wasm32")) && threads).then(|| simthread::SimThread::start(&mut game));

    loop {
        let (sw, sh) = (screen_width(), screen_height());
        let base = View::fit(Rect::new(game.left(), TOP_H, sw - game.left(), sh - TOP_H));
        ui.begin();
        // Screenshots must not depend on where the real pointer happens to be.
        if shot.is_some() {
            ui.freeze(demo_mouse.unwrap_or(vec2(-1000.0, -1000.0)), args.iter().any(|a| a == "--stroke"));
        }
        // The latest picture of the world, or the world itself for a frame that may change it.
        let lent = sim.as_mut().and_then(|s| {
            let edit = simthread::input_may_edit(&game, &ui);
            s.frame(&mut game, edit)
        });
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
        let dt = get_frame_time().min(0.05);
        if sim.is_some() {
            // The world updates in its thread: here, only what belongs to the screen.
            game.tick_toast(dt);
            let mix = game.mix;
            game.sounds.update(dt, &mix);
        } else {
            game.update(dt);
        }
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
            // `--kid x[,act]`: the little boy at x, doing act (walk, wave, jump, flip, wheel, ouch).
            if frame == 357 {
                if let Some(v) = arg("--kid") {
                    let (x, act) = v.split_once(',').unwrap_or((v.as_str(), "walk"));
                    game.kids.demo(&game.world, x.parse().unwrap_or(400.0), act);
                }
            }
            // `--wolves t`: the pack is t seconds into its visit at the shot.
            if let Some(t) = arg("--wolves").and_then(|v| v.parse::<f32>().ok()) {
                if frame == 300 {
                    game.wolves.summon(&game.world);
                }
                if let Some(p) = game.wolves.pack.as_mut().filter(|_| frame >= 355) {
                    p.age = t;
                }
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
            // `--truck-boom n`: the fire engine is set off n frames before the shot.
            if let Some(n) = arg("--truck-boom").and_then(|v| v.parse::<u32>().ok()) {
                if frame + n == 360 {
                    game.blow_up_truck();
                }
            }
        }
        // `--crater n`: the buried powder is lit n frames before the shot.
        if shot.is_some() {
            if let Some(n) = arg("--crater").and_then(|v| v.parse::<u32>().ok()) {
                if frame + n == 360 {
                    game.world.set(480, 359, Mat::Fire);
                    game.world.set(480, 360, Mat::Fire);
                    game.world.set(480, 365, Mat::Fire);
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
        // `--anvil`: a plume of warm damp air off the plain, run until it has built a storm cloud
        // whose top spreads out under the tropopause and is dragged downwind into an anvil.
        if shot.is_some() && args.iter().any(|a| a == "--anvil") {
            // First a long run to build the cloud, then the plume keeps feeding it until the shot.
            let (from, to) = if frame == 1 { (0, 2600u32) } else { (2600 + frame * 4, 2604 + frame * 4) };
            for step in from..to {
                for k in 0..8u32 {
                    let x = 600 + ((step * 7919 + k * 104729) % 120) as i32;
                    let ground = (0..H as i32).find(|&y| !matches!(game.world.at(x, y), Mat::Empty | Mat::Steam | Mat::Cloud | Mat::Smoke)).unwrap_or(H as i32);
                    let y = ground - 2 - ((step + k * 13) % 6) as i32;
                    if game.world.at(x, y) == Mat::Empty {
                        game.world.set(x, y, Mat::Steam);
                        game.world.temp[y as usize * world::W + x as usize] = 32.0;
                    }
                }
                if frame == 1 {
                    game.world.step();
                }
            }
        }
        // `--storm`: a big storm cloud, charged (positive top, negative base), over the valley.
        // `--lightning x`: a bolt from the sky at x, just before the shot.
        if shot.is_some() && frame == 345 && args.iter().any(|a| a == "--storm") {
            for x in 280..680 {
                let top = 70 + ((x as f32 * 0.05).sin() * 8.0) as i32;
                for y in top..200 {
                    game.world.set(x, y, Mat::Cloud);
                }
            }
            for qy in 0..world::QH {
                for qx in 0..world::QW {
                    let (x, y) = ((qx * world::QS) as f32, (qy * world::QS) as f32);
                    let inside = (280.0..680.0).contains(&x) && (70.0..200.0).contains(&y);
                    if inside {
                        let u = ((x - 480.0) / 200.0).powi(2);
                        game.world.charge[qy * world::QW + qx] = (if y < 120.0 { 0.8 } else if y > 150.0 { -0.95 } else { 0.0 }) * (1.0 - u);
                    }
                }
            }
            // The ground under it has had time to take the opposite charge.
            game.world.settle_ground_charge(150);
        }
        if shot.is_some() && frame == 356 {
            // `--lightning x` or `--lightning x,y` (y: where the pointer is; the top of the sky by default).
            if let Some(v) = arg("--lightning") {
                let mut it = v.split(',').filter_map(|n| n.parse::<i32>().ok());
                if let Some(x) = it.next() {
                    game.world.strike(x, it.next().unwrap_or(0));
                }
            }
        }
        // `--bridge`: a truck sets off over the bridge shortly before the shot.
        // `--clouds`: a few fresh clouds across the sky shortly before the shot.
        if shot.is_some() && frame == 330 && args.iter().any(|a| a == "--clouds") {
            for (x, y, r) in [(150, 150, 12), (175, 158, 10), (420, 130, 14), (450, 138, 10), (700, 160, 12), (735, 152, 13), (870, 140, 11)] {
                game.world.paint(x, y, r, Mat::Cloud);
            }
        }
        // `--pour x`: a heap of sand and a ball of water in the air over x, shortly before the shot.
        if shot.is_some() && frame == 320 {
            if let Some(x) = arg("--pour").and_then(|v| v.parse::<i32>().ok()) {
                let y = ground_y(&game.world, x) as i32;
                game.world.paint(x - 14, y - 40, 8, Mat::Sand);
                game.world.paint(x + 14, y - 46, 9, Mat::Water);
            }
        }
        // `--steam x`: a layer of steam on the ground at x, under a layer of water.
        if shot.is_some() && frame == 300 {
            if let Some(x) = arg("--steam").and_then(|v| v.parse::<i32>().ok()) {
                let y = ground_y(&game.world, x);
                for dy in 1..=24 {
                    for dx in -24..=24 {
                        game.world.set(x + dx, y - dy, if dy <= 10 { Mat::Steam } else { Mat::Water });
                    }
                }
            }
        }
        // `--sim-steps n`: the simulation view's step by step, n steps just before the shot.
        if shot.is_some() && frame == 352 {
            if let Some(n) = arg("--sim-steps").and_then(|v| v.parse::<usize>().ok()) {
                game.step_by_step(n);
            }
        }
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
        let t0 = metrics::now();
        game.draw_scene(shaken, &mut renderer, &mut ui);
        let t1 = metrics::now();
        ui.begin_windows(view.visible());
        game.draw_ui(&mut ui, sw, sh);
        ui.end_windows();
        let t2 = metrics::now();
        // The metrics panel (Ctrl+D), over everything: what it reads is the frame just drawn.
        draw_ms = (draw_ms.0 * 0.9 + (t1 - t0) as f32 * 100.0, draw_ms.1 * 0.9 + (t2 - t1) as f32 * 100.0);
        game.draw_metrics(&ui, sw, sh, draw_ms);
        if let (Some(s), Some(world)) = (sim.as_mut(), lent) {
            s.give_back(&mut game, world);
        }

        frame += 1;
        frame_time += get_frame_time();
        if let Some(path) = &shot {
            if frame == arg("--frames").and_then(|v| v.parse().ok()).unwrap_or(360) {
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

    #[test]
    fn a_new_car_is_not_selected_until_clicked() {
        let mut g = mech_game();
        g.entry = Some((100, 290));
        g.launch_car();
        assert_eq!(g.cars.len(), 1);
        assert_eq!(g.selected_car, None, "a car only gets selected when the player clicks it");
    }

    #[test]
    fn the_plan_is_taken_once_and_retry_brings_it_back() {
        let mut g = mech_game();
        g.world.set(200, 100, Mat::Sand);
        g.checkpoint();
        g.blocks.push(Block::new(BlockKind::Brick, vec2(300.0, 200.0), 0.0));
        g.checkpoint();
        assert!(g.building());
        g.set_running(true);
        for _ in 0..60 {
            g.sim_step();
        }
        assert_ne!(g.world.raw(200, 100), Mat::Sand, "the sand fell during the test");
        // A pause, a tinkering and a new start belong to the same test.
        g.set_running(false);
        g.world.set(10, 10, Mat::Stone);
        g.set_running(true);
        g.sim_step();
        g.arrived = 3;
        g.reset();
        assert!(g.building() && !g.running);
        assert_eq!(g.world.raw(200, 100), Mat::Sand, "back to the world as built");
        assert_eq!(g.world.raw(10, 10), Mat::Empty, "the tinkering in the pause is not kept");
        assert_eq!(g.blocks.len(), 1);
        assert_eq!(g.arrived, 0);
        assert_eq!(g.undo.len(), 2, "the building steps are still there to undo");
        // Réessayer on the drawing board changes nothing.
        g.world.set(20, 20, Mat::Stone);
        g.reset();
        assert_eq!(g.world.raw(20, 20), Mat::Stone);
    }

    #[test]
    fn the_challenge_locks_building_and_saves_the_plan() {
        let mut g = mech_game();
        g.world.set(200, 100, Mat::Sand);
        g.set_running(true);
        g.world.set(30, 30, Mat::Stone);
        g.start_challenge();
        assert!(g.challenge && g.running);
        assert_eq!(g.world.raw(30, 30), Mat::Empty, "the challenge starts from the plan, not the test");
        g.checkpoint();
        let n = g.undo.len();
        g.undo();
        assert_eq!(g.undo.len(), n, "no undo during the challenge");
        for _ in 0..60 {
            g.sim_step();
        }
        let plan = g.plan.as_ref().unwrap();
        let bytes = save::encode_cells(g.world.biome, plan.state.clock(), |x, y| World::raw_in(&plan.state, x, y), None, None, &plan.blocks, &plan.cables, &plan.members, &plan.springs, &plan.mech);
        let l = save::decode(&bytes).unwrap();
        assert_eq!(l.mats[100 * W + 200], Mat::Sand, "the save holds the world as built");
        g.reset();
        assert!(!g.challenge && g.building());
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

    #[test]
    fn a_step_by_step_pauses_and_keeps_the_diff() {
        let mut g = Game::new();
        g.world.paint(300, 60, 4, Mat::Sand);
        g.step_by_step(1);
        assert!(!g.running, "the world stays paused after the step");
        let stats = g.simview.stats.as_ref().expect("the step's diff");
        assert_eq!(stats.to, stats.from + 1);
        // The grains in the air fell: their cells changed matter.
        assert!(stats.changed[0].0 > 0);
        g.step_by_step(10);
        let stats = g.simview.stats.as_ref().unwrap();
        assert_eq!(stats.to, stats.from + 10);
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
        for tool in [Tool::Block(BlockKind::Wheel), Tool::Cable(CableMat::Steel), Tool::Spring(SpringKind::Coil), Tool::Beam(MemberMat::Iron, MemberRole::Deck)] {
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
        let hit = |g: &mut Game, at: Vec2| {
            let i = block_at(&g.blocks, at).unwrap();
            g.damage_block(i);
        };
        hit(&mut g, vec2(200.0, 100.0));
        assert_eq!(g.blocks.len(), 2, "stage 1: two halves");
        assert!(g.blocks.iter().all(|b| b.kind == BlockKind::GlassHalf));
        let at = g.blocks[0].pos;
        g.blocks[0].hurt = 0;
        hit(&mut g, at);
        assert_eq!(g.blocks.len(), 5, "stage 2: one half becomes four chips");
        assert_eq!(g.blocks.iter().filter(|b| b.kind == BlockKind::GlassChip).count(), 4);
        let chip = g.blocks.iter().find(|b| b.kind == BlockKind::GlassChip).unwrap().pos;
        for b in &mut g.blocks {
            b.hurt = 0;
        }
        hit(&mut g, chip);
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
