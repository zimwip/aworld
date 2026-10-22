mod backdrop;
mod blocks;
mod car;
mod render;
mod save;
mod science;
mod ui;
mod world;

use blocks::{anchor_at, anchor_world, block_at, clip, node_at, placement_ok, snap, stamp_all, Anchor, Block, BlockKind, Cable, Member, MemberMat, Physics, MEMBER_MAX_LEN};
use car::{collide_cars, Car, Fate, ModelKind, Tuning};
use macroquad::prelude::*;
use render::{cable_preview, draw_block, draw_block_shape, draw_booms, draw_cable, draw_car, draw_flag, draw_member, Boom, Renderer, View};
use science::{info, mat_color, RECIPES};
use ui::{draw_icon, rrect, Icon, Ui, ACCENT, BG, BLUE, DIM, GREEN, PANEL, RED, TEXT};
use world::{Biome, Cell, Mat, Season, World, H, W};

const PANEL_W: f32 = 244.0;
const TOP_H: f32 = 48.0;
/// Width of the sliding views palette on the right, and of its always-visible tab.
const VIEWS_W: f32 = 176.0;
const VIEWS_TAB: f32 = 22.0;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Matter,
    Blocks,
    Cars,
    Tools,
    Planet,
}

#[derive(Clone, Copy, PartialEq)]
enum ViewMode {
    Normal,
    Thermal,
    Stress,
    Wind,
}

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Mat(Mat),
    Block(BlockKind),
    Cable,
    Beam(MemberMat),
    Eraser,
    Rotate,
    Hammer,
    Loupe,
    Pick,
    Entry,
    Exit,
}

const MATTER: [Mat; 19] = [
    Mat::Sand, Mat::Water, Mat::Dirt, Mat::Grass, Mat::Seed, Mat::Wood, Mat::Stone, Mat::Iron, Mat::Steel, Mat::Glass,
    Mat::Ash, Mat::Rust, Mat::Ice, Mat::Snow, Mat::Fire, Mat::Lava, Mat::Steam, Mat::Cloud, Mat::Gunpowder,
];

fn window_conf() -> Conf {
    Conf {
        window_title: "AWorld".to_owned(),
        window_width: (PANEL_W + 1280.0) as i32,
        window_height: (TOP_H + 720.0) as i32,
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
    cars: Vec<Car>,
    arrived: u32,
    lost: u32,
}

struct Game {
    undo: Vec<Snapshot>,
    world: World,
    snap_cells: Vec<Cell>,
    snap_blocks: Vec<Block>,
    snap_cables: Vec<Cable>,
    snap_members: Vec<Member>,
    blocks: Vec<Block>,
    cables: Vec<Cable>,
    members: Vec<Member>,
    cable_from: Option<Anchor>,
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
    hammer_fx: Vec<(Vec2, f32)>,
    time: f32,
    fullscreen: bool,
    mode: ViewMode,
    /// 0 = palette tucked away, 1 = fully out.
    views_open: f32,
    views_pin: bool,
    views_hit: Rect,
    beam_broke: bool,
    /// Edit mode only: building does not use up the world's materials.
    unlimited: bool,
    census: [u32; 32],
    show_science: bool,
    hover_info: Option<(String, String)>,
}

impl Game {
    fn new() -> Self {
        let mut world = World::new();
        let (entry, exit) = world.generate_biome(1, Biome::Countryside);
        Game {
            undo: vec![],
            snap_cells: world.snapshot(),
            snap_blocks: vec![],
            snap_cables: vec![],
            snap_members: vec![],
            world,
            blocks: vec![],
            cables: vec![],
            members: vec![],
            cable_from: None,
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
            hammer_fx: vec![],
            time: 0.0,
            fullscreen: false,
            mode: ViewMode::Normal,
            views_open: 0.0,
            views_pin: false,
            views_hit: Rect::new(-10.0, -10.0, 0.0, 0.0),
            beam_broke: false,
            unlimited: true,
            census: [0; 32],
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
        self.cars = s.cars;
        self.arrived = s.arrived;
        self.lost = s.lost;
        self.cable_from = None;
        self.booms.clear();
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
        for c in &mut self.cables {
            c.reset_runtime();
            p.add_cable(&self.blocks, c);
        }
        self.phys = Some(p);
    }

    fn set_running(&mut self, run: bool) {
        if run && !self.running {
            self.snap_cells = self.world.snapshot();
            self.snap_blocks = self.blocks.iter().map(|b| Block { body: None, ..b.clone() }).collect();
            self.snap_cables = self.cables.iter().map(Cable::fresh).collect();
            self.snap_members = self.members.iter().map(Member::fresh).collect();
            if self.phys.is_none() {
                self.build_physics();
            }
        }
        self.running = run;
    }

    fn reset(&mut self) {
        self.world.restore(&self.snap_cells);
        self.blocks = self.snap_blocks.clone();
        self.cables = self.snap_cables.iter().map(Cable::fresh).collect();
        self.members = self.snap_members.iter().map(Member::fresh).collect();
        self.cable_from = None;
        self.member_from = None;
        self.phys = None;
        self.cars.clear();
        self.running = false;
        self.refresh_mask();
    }

    fn fresh(&mut self) {
        self.undo.clear();
        self.blocks.clear();
        self.cables.clear();
        self.members.clear();
        self.cable_from = None;
        self.member_from = None;
        self.phys = None;
        self.cars.clear();
        self.selected_car = None;
        self.running = false;
        self.arrived = 0;
        self.lost = 0;
        self.refresh_mask();
        self.snap_cells = self.world.snapshot();
        self.snap_blocks.clear();
        self.snap_cables.clear();
        self.snap_members.clear();
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
            self.say("Pose d'abord une Entrée (onglet Outils)");
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
        match save::save_slot(self.slot, &self.world, self.entry, self.exit, &self.blocks, &self.cables, &self.members) {
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
                self.snap_blocks = self.blocks.clone();
                self.snap_cables = self.cables.iter().map(Cable::fresh).collect();
                self.snap_members = self.members.iter().map(Member::fresh).collect();
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
        let free = self.unlimited && !self.running;
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

    fn remove_block(&mut self, i: usize) {
        let uid = self.blocks[i].uid;
        self.remove_cables_of(uid);
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
        let uid = b.uid;
        if let Some(p) = &mut self.phys {
            p.remove_block(&mut self.blocks[i]);
            b.body = None;
            p.add_block(&mut b);
        }
        self.blocks[i] = b;
        if let Some(p) = &mut self.phys {
            p.weld_neighbours(&self.blocks, i);
            for c in self.cables.iter_mut().filter(|c| c.attached_to(uid)) {
                p.remove_cable(c);
                p.add_cable(&self.blocks, c);
            }
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
        self.booms.push(Boom { pos: b.pos, t: 0.6, color: Color::new(0.7, 0.9, 1.0, 1.0) });
    }

    fn break_block(&mut self, i: usize) {
        let pos = self.blocks[i].pos;
        self.crumble(i, Mat::Shard, true);
        self.booms.push(Boom { pos, t: 0.55, color: Color::new(0.7, 0.9, 1.0, 1.0) });
    }

    fn hurt_cooldown_clear(&mut self, i: usize) {
        self.blocks[i].hurt = 0;
    }

    fn burn_block(&mut self, i: usize) {
        self.crumble(i, Mat::Ember, false);
    }

    fn cable_click(&mut self, at: Vec2) {
        let Some(anchor) = anchor_at(&self.world, &self.blocks, at) else {
            self.say("Accroche le câble sur un bloc ou sur le sol");
            return;
        };
        match self.cable_from.take() {
            None => self.cable_from = Some(anchor),
            Some(first) => {
                let (Some(a), Some(b)) = (anchor_world(&first, &self.blocks, &self.members), anchor_world(&anchor, &self.blocks, &self.members)) else { return };
                let chord = (b - a).length();
                if chord < 10.0 {
                    self.say("Tire le câble plus loin avant de l'accrocher");
                    self.cable_from = Some(first);
                    return;
                }
                self.checkpoint();
                let mut c = Cable::new(first, anchor, chord);
                if let Some(p) = &mut self.phys {
                    p.add_cable(&self.blocks, &mut c);
                }
                self.cables.push(c);
            }
        }
    }

    fn cable_points(&self, c: &Cable) -> Vec<Vec2> {
        if !c.pts.is_empty() {
            return c.pts.clone();
        }
        match (anchor_world(&c.a, &self.blocks, &self.members), anchor_world(&c.b, &self.blocks, &self.members)) {
            (Some(a), Some(b)) => cable_preview(a, b, c.len),
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

    fn remove_member_at(&mut self, i: usize) {
        let mut m = self.members.remove(i);
        if let Some(p) = &mut self.phys {
            p.remove_member(&mut m);
        }
        self.fix_dangling_anchors();
        self.refresh_mask();
    }

    /// Where the far end of a beam started at `p0` would land for a cursor at `at`:
    /// on a nearby node if there is one, otherwise free, never longer than the maximum.
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
                None => self.say("Commence sur le sol, un bloc ou un nœud de poutre"),
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
                let free = self.unlimited && !self.running;
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
        if is_key_pressed(KeyCode::Escape) {
            if self.show_science {
                self.show_science = false;
            } else {
                self.cable_from = None;
                self.member_from = None;
            }
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
                Tab::Tools => Tab::Planet,
                Tab::Planet => Tab::Matter,
            };
        }
        let digits = [
            KeyCode::Key1, KeyCode::Key2, KeyCode::Key3, KeyCode::Key4, KeyCode::Key5,
            KeyCode::Key6, KeyCode::Key7, KeyCode::Key8, KeyCode::Key9, KeyCode::Key0,
        ];
        for (i, k) in digits.iter().enumerate() {
            if is_key_pressed(*k) {
                self.tool = Tool::Mat(MATTER[i]);
                self.tab = Tab::Matter;
            }
        }
    }

    fn handle_mouse(&mut self, view: View, ui: &Ui) {
        if self.show_science {
            return;
        }
        let over_ui = ui.mouse.x < PANEL_W || ui.mouse.y < TOP_H || self.views_hit.contains(ui.mouse);
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

        if rclick && (self.cable_from.is_some() || self.member_from.is_some()) {
            self.cable_from = None;
            self.member_from = None;
            return;
        }
        if right || (left && self.tool == Tool::Eraser) {
            if click || rclick {
                if let Some(i) = block_at(&self.blocks, at) {
                    self.remove_block(i);
                    return;
                }
                if let Some(i) = self.members.iter().position(|m| m.distance_to(at) < m.mat.thickness() / 2.0 + 3.0) {
                    self.remove_member_at(i);
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
            Tool::Mat(m) => self.world.line(from, g, self.brush, m),
            Tool::Block(k) if click => self.place_block(k, at),
            Tool::Cable if click => self.cable_click(at),
            Tool::Beam(m) if click => self.beam_click(m, at),
            Tool::Hammer if click => self.hammer(at),
            Tool::Rotate if click => {
                if let Some(i) = block_at(&self.blocks, at) {
                    self.rotate_block(i);
                }
            }
            Tool::Pick if click => {
                self.selected_car = self.cars.iter().find(|c| (c.p - at).length() < c.kind.model().hull_half * car::S + 10.0).map(|c| c.id);
                if let Some(id) = self.selected_car {
                    if let Some(c) = self.cars.iter().find(|c| c.id == id) {
                        self.model = c.kind;
                    }
                    self.tab = Tab::Cars;
                } else {
                    self.say("Clique sur une voiture en piste");
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
        if self.tab == Tab::Blocks && (self.time * 4.0) as u32 != ((self.time - dt) * 4.0) as u32 {
            self.census = self.world.census();
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
        // Cars that meet either bump gently or wreck each other on the spot.
        for i in 0..self.cars.len() {
            for j in i + 1..self.cars.len() {
                let (left, right) = self.cars.split_at_mut(j);
                collide_cars(&mut left[i], &mut right[0]);
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
            p.sync_cables(&self.blocks, &mut self.cables);
            for spot in p.update_members(&self.world, &mut self.members) {
                self.booms.push(Boom { pos: spot, t: 0.5, color: Color::new(0.9, 0.7, 0.4, 1.0) });
                self.beam_broke = true;
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
            self.fix_dangling_anchors();
        }
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
            self.booms.push(Boom { pos: c, t: 0.0, color: Color::new(1.0, 0.5, 0.1, 1.0) });
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
        if !thermal {
            backdrop::draw_sky(view, self.world.biome, phase, self.world.wind, self.time, self.world.season());
        }
        renderer.draw_world(&self.world, view, thermal);
        if !thermal {
            renderer.draw_gas(&self.world, view);
        }
        let stress = if self.mode == ViewMode::Stress { self.phys.as_ref().map(|p| p.stress(&self.blocks)) } else { None };
        for b in &self.blocks {
            let st = stress.as_ref().map(|m| m.get(&b.uid).copied().unwrap_or(0.0));
            draw_block(b, view, st);
        }
        for m in &self.members {
            draw_member(m, view, self.mode == ViewMode::Stress);
        }
        for c in &self.cables {
            let pts = self.cable_points(c);
            draw_cable(&pts, view, c.snapped, true);
        }
        if let Some(e) = self.entry {
            draw_flag(view, e, GREEN, false);
        }
        if let Some(x) = self.exit {
            draw_flag(view, x, RED, true);
        }
        for c in &self.cars {
            draw_car(c, view);
            if self.selected_car == Some(c.id) {
                let s = view.to_screen(c.p);
                draw_circle_lines(s.x, s.y, (c.kind.model().hull_half * car::S + 8.0) * view.s, 2.0, ACCENT);
            }
        }
        draw_booms(&self.booms, view);
        for (at, t) in &self.hammer_fx {
            let s = view.to_screen(*at);
            let angle = -1.1 + 1.6 * t;
            let head = s + vec2(angle.sin(), -angle.cos()) * 34.0;
            draw_line(s.x, s.y, head.x, head.y, 4.0, Color::from_rgba(150, 100, 60, 255));
            draw_circle(head.x, head.y, 9.0, Color::from_rgba(120, 124, 140, 255));
            draw_circle_lines(s.x, s.y, 6.0 + 26.0 * t, 2.0, Color::new(1.0, 1.0, 1.0, 1.0 - t));
        }
        if !thermal {
            backdrop::draw_night(view, day);
        }
        if self.mode == ViewMode::Wind {
            self.draw_wind_field(view);
        }

        // Cursor preview.
        let over_ui = ui.mouse.x < PANEL_W || ui.mouse.y < TOP_H || self.views_hit.contains(ui.mouse);
        let inside = !over_ui && view.rect().contains(ui.mouse) && !self.show_science;
        if let Some(a) = &self.cable_from {
            if let Some(p) = anchor_world(a, &self.blocks, &self.members) {
                let end = if inside { view.to_world(ui.mouse) } else { p };
                let ok = inside && anchor_at(&self.world, &self.blocks, end).is_some();
                let pts = cable_preview(p, end, (end - p).length());
                draw_cable(&pts, view, false, ok);
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
        if inside {
            let at = view.to_world(ui.mouse);
            match self.tool {
                Tool::Block(k) => {
                    let cand = self.candidate(k, at);
                    let ok = placement_ok(&self.world, &self.blocks, &cand, None);
                    let tint = if ok { Color::new(0.3, 1.0, 0.5, 1.0) } else { Color::new(1.0, 0.3, 0.3, 1.0) };
                    draw_block_shape(k, view.to_screen(cand.pos), view.s, cand.angle, Some(tint), 0.0, 0.0, None);
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
                Tool::Beam(_) => {
                    let ok = node_at(&self.world, &self.blocks, &self.members, at).is_some() || self.member_from.is_some();
                    draw_circle_lines(ui.mouse.x, ui.mouse.y, 5.0 * view.s, 2.0, if ok { GREEN } else { RED });
                }
                Tool::Cable => {
                    let ok = anchor_at(&self.world, &self.blocks, at).is_some();
                    let col = if ok { GREEN } else { RED };
                    draw_circle_lines(ui.mouse.x, ui.mouse.y, 5.0 * view.s, 2.0, col);
                }
                Tool::Hammer => draw_circle_lines(ui.mouse.x, ui.mouse.y, 3.5 * view.s, 1.5, ACCENT),
                Tool::Loupe => self.draw_loupe(ui, view, at),
                _ => {}
            }
        }

        self.draw_hud(ui, view);
    }

    fn draw_loupe(&self, ui: &Ui, view: View, at: Vec2) {
        let (x, y) = (at.x as i32, at.y as i32);
        let (title, body) = if let Some(i) = block_at(&self.blocks, at) {
            let b = &self.blocks[i];
            let s = b.spec();
            let rust = if b.rust > 0.02 { format!(" · rouille {:.0} %", b.rust * 100.0) } else { String::new() };
            (format!("{} (bloc){rust}", s.name), s.blurb.to_string())
        } else {
            let m = self.world.raw(x, y);
            let i = info(m);
            (format!("{} · {} · {:.0} °C", i.name, i.state, self.world.temp_c(x, y)), i.fact.to_string())
        };
        let w = 300.0;
        let lines = ui.wrapped_height(&body, w - 20.0, 15);
        let mut r = Rect::new(ui.mouse.x + 18.0, ui.mouse.y + 18.0, w, 38.0 + lines);
        let screen = view.rect();
        if r.x + r.w > screen.x + screen.w {
            r.x = ui.mouse.x - r.w - 18.0;
        }
        if r.y + r.h > screen.y + screen.h {
            r.y = ui.mouse.y - r.h - 18.0;
        }
        rrect(r, 10.0, Color::new(0.06, 0.07, 0.12, 0.94));
        ui.text(&title, r.x + 10.0, r.y + 22.0, 17, ACCENT);
        ui.wrapped(&body, r.x + 10.0, r.y + 42.0, w - 20.0, 15, TEXT);
    }

    fn draw_hud(&self, ui: &Ui, view: View) {
        let r = view.rect();
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
        rrect(pill, 13.0, Color::new(0.0, 0.0, 0.0, 0.5));
        draw_icon(if night { Icon::Moon } else { Icon::Sun }, vec2(pill.x + 16.0, pill.y + 13.0), 17.0, if night { Color::new(0.85, 0.9, 1.0, 1.0) } else { ACCENT });
        ui.text(&text, pill.x + 30.0, pill.y + 18.0, 14, WHITE);

        let stats = format!("Arrivées {}   Perdues {}", self.arrived, self.lost);
        let sw = ui.text_w(&stats, 14) + 22.0;
        let sp = Rect::new(r.x + 8.0, r.y + r.h - 32.0, sw, 24.0);
        rrect(sp, 12.0, Color::new(0.0, 0.0, 0.0, 0.5));
        ui.text(&stats, sp.x + 11.0, sp.y + 17.0, 14, WHITE);

        let (label, col) = if self.running { ("SIMULATION", RED) } else { ("CONSTRUCTION", BLUE) };
        let bw = ui.text_w(label, 14) + 32.0;
        rrect(Rect::new(r.x + r.w - bw - 8.0, r.y + 8.0, bw, 26.0), 13.0, Color::new(0.0, 0.0, 0.0, 0.5));
        draw_circle(r.x + r.w - bw + 6.0, r.y + 21.0, 5.0, col);
        ui.text(label, r.x + r.w - bw + 16.0, r.y + 26.0, 14, WHITE);
    }

    fn draw_ui(&mut self, ui: &mut Ui, sw: f32, sh: f32) {
        self.hover_info = None;
        draw_rectangle(0.0, 0.0, sw, TOP_H, BG);
        draw_rectangle(0.0, TOP_H, PANEL_W, sh - TOP_H, PANEL);
        draw_rectangle(0.0, TOP_H - 2.0, sw, 2.0, Color::new(0.0, 0.0, 0.0, 0.35));

        self.draw_top_bar(ui, sw);
        self.draw_panel(ui, sh);
        self.draw_info_box(ui, sh);
        self.draw_views(ui, sw, sh);
        if self.show_science {
            self.draw_science(ui, sw, sh);
        }

        if let Some((msg, t)) = &self.toast {
            let a = (*t / 0.5).min(1.0);
            let w = ui.text_w(msg, 20) + 40.0;
            let r = Rect::new(PANEL_W + (sw - PANEL_W - w) / 2.0, sh - 76.0, w, 42.0);
            rrect(r, 21.0, Color::new(0.05, 0.06, 0.1, 0.92 * a));
            ui.text(msg, r.x + 20.0, r.y + 27.0, 20, Color::new(1.0, 1.0, 1.0, a));
        }
    }

    fn draw_top_bar(&mut self, ui: &mut Ui, sw: f32) {
        #[derive(Clone, Copy, PartialEq)]
        enum B {
            Play,
            Reset,
            Undo,
            Car,
            Slower,
            Faster,
            Rotate,
            Slot(usize),
            Save,
            Load,
            World,
            Blank,
            Science,
            Full,
        }
        let items: [(f32, B); 14] = [
            (104.0, B::Play),
            (100.0, B::Reset),
            (104.0, B::Undo),
            (108.0, B::Car),
            (38.0, B::Slower),
            (50.0, B::Faster),
            (104.0, B::Rotate),
            (32.0, B::Slot(0)),
            (32.0, B::Slot(1)),
            (32.0, B::Slot(2)),
            (92.0, B::Save),
            (100.0, B::Load),
            (88.0, B::World),
            (70.0, B::Blank),
        ];
        let tail: [(f32, B); 2] = [(98.0, B::Science), (70.0, B::Full)];
        let all: Vec<(f32, B)> = items.iter().chain(tail.iter()).copied().collect();
        let gap = 6.0;
        let total: f32 = all.iter().map(|i| i.0).sum::<f32>() + gap * (all.len() as f32 + 1.0) + 40.0;
        let k = (sw / total).min(1.0) * 0.86;
        let mut x = 8.0;
        let h = 32.0;
        let y = (TOP_H - h) / 2.0 - 1.0;
        for (w, b) in all {
            let w = w * k;
            let r = Rect::new(x, y, w, h);
            x += w + gap * k;
            if matches!(b, B::Slower) {
                x -= gap * k * 0.5;
            }
            match b {
                B::Play => {
                    let (label, icon) = if self.running { ("Pause", Icon::Pause) } else { ("Jouer", Icon::Play) };
                    if ui.button(r, label, icon, !self.running, true, GREEN) {
                        let run = !self.running;
                        self.set_running(run);
                    }
                }
                B::Reset => {
                    if ui.button(r, "Retour", Icon::Reset, false, true, ACCENT) {
                        self.reset();
                    }
                }
                B::Undo => {
                    let label = if self.undo.is_empty() { "Annuler".to_string() } else { format!("Annuler {}", self.undo.len()) };
                    if ui.button(r, &label, Icon::Undo, false, !self.undo.is_empty(), BLUE) {
                        self.undo();
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
                    if ui.button(r, "", Icon::Plus, false, self.speed < 6, ACCENT) {
                        self.speed += 1;
                    }
                    ui.text_center(&format!("x{}", self.speed), r.x + r.w + 13.0 * k, r.y + h / 2.0 + 6.0, 16, DIM);
                    x += 26.0 * k;
                }
                B::Rotate => {
                    if ui.button(r, "Tourner", Icon::Rotate, matches!(self.tool, Tool::Block(_)) && self.rot90 != 0, true, BLUE) {
                        self.rotate_preview(1);
                    }
                }
                B::Slot(i) => {
                    let used = save::slot_used(i);
                    if ui.button(r, &format!("{}", i + 1), Icon::None, self.slot == i, true, if used { ACCENT } else { DIM }) {
                        self.slot = i;
                    }
                    if used && self.slot != i {
                        draw_circle(r.x + r.w - 6.0, r.y + 6.0, 3.5, ACCENT);
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
                        let b = Biome::ALL[(self.world.biome.id() as usize + 1) % 3];
                        self.new_world(b);
                    }
                }
                B::Blank => {
                    if ui.button(r, "Vide", Icon::Blank, false, true, ACCENT) {
                        self.empty_world();
                    }
                }
                B::Science => {
                    if ui.button(r, "Science", Icon::Science, self.show_science, true, BLUE) {
                        self.show_science = !self.show_science;
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

    fn tile_rect(i: usize, cols: usize, top: f32, h: f32) -> Rect {
        let gap = 6.0;
        let w = (PANEL_W - gap * (cols as f32 + 1.0)) / cols as f32;
        Rect::new(gap + (i % cols) as f32 * (w + gap), top + (i / cols) as f32 * (h + gap), w, h)
    }

    fn draw_panel(&mut self, ui: &mut Ui, sh: f32) {
        let tabs = [(Tab::Matter, "Matières"), (Tab::Blocks, "Blocs"), (Tab::Cars, "Voitures"), (Tab::Tools, "Outils"), (Tab::Planet, "Monde")];
        let tw = (PANEL_W - 6.0 * 4.0) / 3.0;
        for (i, (tab, label)) in tabs.iter().enumerate() {
            let r = Rect::new(6.0 + (i % 3) as f32 * (tw + 6.0), TOP_H + 8.0 + (i / 3) as f32 * 38.0, tw, 32.0);
            if ui.button(r, label, Icon::None, self.tab == *tab, true, ACCENT) {
                self.tab = *tab;
            }
        }
        let top = TOP_H + 6.0 + 62.0 + 6.0;
        match self.tab {
            Tab::Matter => self.panel_matter(ui, top),
            Tab::Blocks => self.panel_blocks(ui, top),
            Tab::Cars => self.panel_cars(ui, top),
            Tab::Tools => self.panel_tools(ui, top),
            Tab::Planet => self.panel_planet(ui, top),
        }
        let _ = sh;
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

        let grid_top = top + 34.0;
        for (i, m) in MATTER.iter().enumerate() {
            let r = Self::tile_rect(i, 3, grid_top, 48.0);
            let inf = info(*m);
            let col = mat_color(*m);
            let hot = ui.hover(r);
            if hot {
                self.hover_info = Some((format!("{} · {}", inf.name, inf.state), inf.fact.to_string()));
            }
            if ui.tile(r, inf.name, self.tool == Tool::Mat(*m), |s| {
                rrect(s, 7.0, col);
                rrect(Rect::new(s.x, s.y, s.w, s.h * 0.4), 7.0, Color::new(1.0, 1.0, 1.0, 0.18));
                if matches!(m, Mat::Fire | Mat::Lava) {
                    draw_circle(s.x + s.w * 0.5, s.y + s.h * 0.5, s.h * 0.32, Color::new(1.0, 0.9, 0.3, 0.55));
                }
            }) {
                self.tool = Tool::Mat(*m);
            }
        }
    }

    fn stock(&self, from: &[Mat]) -> u32 {
        from.iter().map(|m| self.census[*m as usize]).sum()
    }

    fn panel_blocks(&mut self, ui: &mut Ui, top: f32) {
        let angle = self.rot90 as f32 * std::f32::consts::FRAC_PI_2;
        let free = self.unlimited && !self.running;
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
                rrect(r, 12.0, Color::new(0.7, 0.1, 0.1, 0.38));
            }
        }
        let i = BlockKind::PLACEABLE.len();
        let r = Self::tile_rect(i, 3, top, 54.0);
        if ui.hover(r) {
            self.hover_info = Some(("Câble".into(), "Clique un premier point, tire, clique un second point : la physique fait le reste. Il casse si on tire trop fort.".into()));
        }
        if ui.tile(r, "Câble", self.tool == Tool::Cable, |s| {
            draw_icon(Icon::Cable, vec2(s.x + s.w / 2.0, s.y + s.h / 2.0), 30.0, TEXT);
        }) {
            self.tool = Tool::Cable;
        }

        // Free beams: pinned at both ends, like a bridge-building game.
        for (k, mat) in MemberMat::ALL.iter().enumerate() {
            let r = Self::tile_rect(i + 1 + k, 3, top, 54.0);
            let (units, from) = (mat.cost(60.0), mat.cost_from());
            if ui.hover(r) {
                self.hover_info = Some((
                    format!("Tige {}", mat.name().to_lowercase()),
                    format!(
                        "Un clic pour fixer un bout (sol, bloc ou nœud), tire, un second clic pour l'autre bout. Longueur max {:.0}. 60 de long coûte {units} de {}.",
                        MEMBER_MAX_LEN,
                        info(from[0]).name.to_lowercase()
                    ),
                ));
            }
            let (cr, cg, cb) = mat.color();
            let label = format!("Tige {}", mat.name().to_lowercase());
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
        }

        // Materials: a block uses up what it is made of from the screen.
        let y0 = top + 4.0 * 60.0 + 8.0;
        let label = if self.running { "Matériaux limités (simulation)" } else if self.unlimited { "Matériaux illimités" } else { "Matériaux limités" };
        if ui.button(Rect::new(6.0, y0, PANEL_W - 12.0, 26.0), label, Icon::None, free, !self.running, GREEN) {
            self.unlimited = !self.unlimited;
        }
        let rows: [(&str, &[Mat]); 6] = [
            ("Terre", &[Mat::Dirt]),
            ("Bois", &[Mat::Wood, Mat::Trunk, Mat::Branch]),
            ("Fer", &[Mat::Iron]),
            ("Acier", &[Mat::Steel]),
            ("Pierre", &[Mat::Stone]),
            ("Sable", &[Mat::Sand]),
        ];
        for (i, (name, mats)) in rows.iter().enumerate() {
            let y = y0 + 34.0 + i as f32 * 19.0;
            let n = self.stock(mats);
            let col = mat_color(mats[0]);
            ui.text(name, 10.0, y + 11.0, 13, DIM);
            let bar = Rect::new(66.0, y + 2.0, PANEL_W - 132.0, 10.0);
            rrect(bar, 6.0, Color::new(0.0, 0.0, 0.0, 0.35));
            rrect(Rect::new(bar.x, bar.y, (bar.w * (n as f32 / 3000.0).min(1.0)).max(if n > 0 { 6.0 } else { 0.0 }), bar.h), 6.0, col);
            ui.text(&format!("{n}"), bar.x + bar.w + 8.0, y + 11.0, 12, TEXT);
        }
        ui.text("Peins de la matière pour en avoir.", 10.0, y0 + 34.0 + 6.0 * 19.0 + 10.0, 12, DIM);
    }

    fn panel_tools(&mut self, ui: &mut Ui, top: f32) {
        let tools: [(Tool, &str, Icon, &str); 8] = [
            (Tool::Eraser, "Gomme", Icon::Eraser, "Efface la matière, les blocs et les câbles (clic droit aussi)."),
            (Tool::Rotate, "Tourner", Icon::Rotate, "Clique un bloc posé pour le pivoter d'un quart de tour."),
            (Tool::Hammer, "Marteau", Icon::Hammer, "Tape sur le verre et la glace pour les casser, secoue les autres blocs."),
            (Tool::Loupe, "Loupe", Icon::Loupe, "Survole le monde : nom, état, température et explication."),
            (Tool::Pick, "Voiture", Icon::Pick, "Clique une voiture en piste pour régler ses caractéristiques."),
            (Tool::Entry, "Entrée", Icon::Flag, "Où partent les voitures."),
            (Tool::Exit, "Sortie", Icon::Flag, "Où elles doivent arriver."),
            (Tool::Cable, "Câble", Icon::Cable, "Accroche un câble entre deux points."),
        ];
        for (i, (t, label, icon, text)) in tools.iter().enumerate() {
            let r = Self::tile_rect(i, 3, top, 54.0);
            if ui.hover(r) {
                self.hover_info = Some((label.to_string(), text.to_string()));
            }
            let col = if *t == Tool::Exit { RED } else if *t == Tool::Entry { GREEN } else { TEXT };
            if ui.tile(r, label, self.tool == *t, |s| {
                draw_icon(*icon, vec2(s.x + s.w / 2.0, s.y + s.h / 2.0), 30.0, col);
            }) {
                self.tool = *t;
            }
        }
    }

    fn panel_cars(&mut self, ui: &mut Ui, top: f32) {
        for (i, k) in ModelKind::ALL.iter().enumerate() {
            let m = k.model();
            let r = Rect::new(6.0, top + i as f32 * 40.0, PANEL_W - 12.0, 36.0);
            let sel = self.model == *k;
            let hot = ui.hover(r);
            rrect(r, 10.0, if hot { ui::TILE_HOT } else { ui::TILE });
            if sel {
                ui::rrect_lines(r, 10.0, 3.0, ACCENT);
            }
            let c = self.tunings[i].color.unwrap_or(m.color);
            let c = Color::from_rgba(c.0, c.1, c.2, 255);
            rrect(Rect::new(r.x + 7.0, r.y + 16.0, 34.0, 10.0), 3.0, c);
            rrect(Rect::new(r.x + 13.0, r.y + 8.0, 18.0, 10.0), 3.0, c);
            draw_circle(r.x + 14.0, r.y + 28.0, 5.0, Color::from_rgba(26, 26, 32, 255));
            draw_circle(r.x + 34.0, r.y + 28.0, 5.0, Color::from_rgba(26, 26, 32, 255));
            ui.text(m.name, r.x + 50.0, r.y + 16.0, 16, if sel { ACCENT } else { TEXT });
            ui.text(m.blurb, r.x + 50.0, r.y + 30.0, 12, DIM);
            if hot && ui.pressed {
                self.model = *k;
                self.selected_car = None;
            }
        }

        let sy = top + 4.0 * 40.0 + 6.0;
        let editing_live = self.selected_car.map_or(false, |id| self.cars.iter().any(|c| c.id == id));
        let name = self.model.model().name;
        ui.text(&format!("Réglages · {name}"), 10.0, sy + 11.0, 15, ACCENT);
        ui.text(if editing_live { "voiture en piste" } else { "prochaines voitures" }, 10.0, sy + 25.0, 11, DIM);
        let rows: [(&str, f32, f32, usize); 8] = [
            ("Puissance", 0.5, 2.0, 0),
            ("Vitesse max", 0.5, 2.0, 1),
            ("Suspension", 0.5, 2.0, 2),
            ("Amortisseurs", 0.5, 2.0, 3),
            ("Adhérence", 0.6, 1.6, 4),
            ("Roues", 0.7, 1.5, 5),
            ("Solidité", 0.6, 2.0, 6),
            ("Poids", 0.5, 3.0, 7),
        ];
        let mut changed = false;
        for (i, (label, lo, hi, which)) in rows.iter().enumerate() {
            let y = sy + 32.0 + i as f32 * 25.0;
            let mut t = *self.tuning_mut();
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
            let hover = ui.hover(Rect::new(0.0, y - 3.0, PANEL_W, 24.0));
            ui.text(label, 10.0, y + 11.0, 13, if hover { TEXT } else { DIM });
            let before = *v;
            ui.slider(Rect::new(98.0, y, PANEL_W - 148.0, 18.0), v, *lo, *hi, ACCENT);
            ui.text(&format!("x{:.1}", *v), PANEL_W - 44.0, y + 11.0, 13, TEXT);
            if (*v - before).abs() > 1e-4 {
                *self.tuning_mut() = t;
                changed = true;
            }
        }
        // Colour swatches.
        let cy = sy + 32.0 + 8.0 * 25.0 + 2.0;
        for (i, c) in Tuning::COLORS.iter().enumerate() {
            let cx = 16.0 + i as f32 * 26.0;
            let r = Rect::new(cx - 10.0, cy, 22.0, 22.0);
            let chosen = self.tuning_mut().color == Some(*c);
            draw_circle(cx, cy + 11.0, 9.5, Color::from_rgba(c.0, c.1, c.2, 255));
            if chosen {
                draw_circle_lines(cx, cy + 11.0, 12.0, 2.0, WHITE);
            }
            if ui.hover(r) && ui.pressed {
                self.tuning_mut().color = Some(*c);
                changed = true;
            }
        }
        let rr = Rect::new(8.0, cy + 28.0, PANEL_W - 16.0, 26.0);
        if ui.button(rr, "Réinitialiser", Icon::Reset, false, true, ACCENT) {
            *self.tuning_mut() = Tuning::default();
            changed = true;
        }
        if changed {
            self.sync_default_tuning();
            if !editing_live {
                // nothing else to do: next cars use it
            }
        }
    }

    fn panel_planet(&mut self, ui: &mut Ui, top: f32) {
        ui.text("Décor du monde", 10.0, top + 14.0, 16, ACCENT);
        for (i, b) in Biome::ALL.iter().enumerate() {
            let r = Rect::new(6.0, top + 22.0 + i as f32 * 32.0, PANEL_W - 12.0, 28.0);
            if ui.button(r, b.name(), Icon::World, self.world.biome == *b, true, GREEN) {
                self.new_world(*b);
            }
        }
        let ty = top + 22.0 + 3.0 * 32.0 + 12.0;
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
        ui.text(&format!("Air au sol : {:.0} °C", w.surface_temp_c()), 10.0, iy, 15, TEXT);
        ui.text(&format!("Vent : {:.0} km/h {}", w.wind.abs() * 40.0, if w.wind > 0.0 { "vers la droite" } else { "vers la gauche" }), 10.0, iy + 20.0, 15, TEXT);
        ui.text("Le soleil chauffe le jour, la nuit gèle.", 10.0, iy + 44.0, 13, DIM);
        ui.text("Touche H : vue thermique.", 10.0, iy + 62.0, 13, DIM);
    }

    /// The palette of views on the right edge: it slides out when the mouse goes there.
    fn draw_views(&mut self, ui: &mut Ui, sw: f32, sh: f32) {
        let y = TOP_H + 60.0;
        let h = 252.0;
        let tab = Rect::new(sw - VIEWS_TAB, y, VIEWS_TAB, 96.0);
        let px = sw - VIEWS_TAB - VIEWS_W * self.views_open;
        let panel = Rect::new(px, y, VIEWS_W, h);
        let near_edge = ui.mouse.x > sw - 8.0 && ui.mouse.y > TOP_H;
        let hovering = tab.contains(ui.mouse) || (self.views_open > 0.05 && panel.contains(ui.mouse)) || near_edge;
        let target = if hovering || self.views_pin { 1.0 } else { 0.0 };
        self.views_open += (target - self.views_open) * (get_frame_time() * 9.0).min(1.0);
        if (self.views_open - target).abs() < 0.01 {
            self.views_open = target;
        }
        self.views_hit = if self.views_open > 0.02 { tab.combine_with(panel) } else { tab };

        // The tab that stays visible.
        rrect(tab, 8.0, Color::new(0.1, 0.12, 0.2, 0.92));
        draw_icon(Icon::Eye, vec2(tab.x + tab.w / 2.0, tab.y + 22.0), 15.0, if self.mode == ViewMode::Normal { DIM } else { ACCENT });
        let arrow = if self.views_open > 0.5 { "›" } else { "‹" };
        ui.text_center(arrow, tab.x + tab.w / 2.0, tab.y + 56.0, 20, TEXT);
        if self.mode != ViewMode::Normal {
            draw_circle(tab.x + tab.w / 2.0, tab.y + 78.0, 4.0, ACCENT);
        }
        if self.views_open <= 0.02 {
            return;
        }

        rrect(panel, 12.0, Color::new(0.09, 0.1, 0.17, 0.95));
        ui.text("Vues", panel.x + 12.0, panel.y + 22.0, 16, ACCENT);
        let modes: [(ViewMode, &str, Icon, Color); 4] = [
            (ViewMode::Normal, "Normale", Icon::Sun, ACCENT),
            (ViewMode::Thermal, "Température", Icon::Thermo, RED),
            (ViewMode::Stress, "Efforts", Icon::Gauge, ACCENT),
            (ViewMode::Wind, "Vent", Icon::Wind, BLUE),
        ];
        for (i, (m, label, icon, col)) in modes.iter().enumerate() {
            let r = Rect::new(panel.x + 8.0, panel.y + 32.0 + i as f32 * 31.0, panel.w - 16.0, 27.0);
            if ui.button(r, label, *icon, self.mode == *m, true, *col) {
                self.mode = *m;
            }
        }
        // Legend for the chosen view.
        let ly = panel.y + 32.0 + 4.0 * 31.0 + 8.0;
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
                ui.wrapped("Lance la simulation : les pièces chargées virent au rouge.", bar.x, bar.y + 42.0, bar.w, 11, DIM);
            }
            ViewMode::Wind => {
                let w = &self.world;
                ui.text(&format!("{:.0} km/h {}", w.wind.abs() * 40.0, if w.wind > 0.0 { "→" } else { "←" }), bar.x, bar.y + 4.0, 14, TEXT);
                ui.wrapped("Plus fort en altitude : il pousse nuages, vapeur et pluie.", bar.x, bar.y + 22.0, bar.w, 11, DIM);
            }
            ViewMode::Normal => {
                ui.wrapped("Touches : H température, X efforts, V vent.", bar.x, bar.y + 2.0, bar.w, 11, DIM);
            }
        }
        let _ = sh;
    }

    /// Arrows showing the wind: longer and stronger higher in the sky.
    fn draw_wind_field(&self, view: View) {
        let r = view.rect();
        let w = self.world.wind;
        let (cols, rows) = (24, 12);
        for j in 0..rows {
            let alt = 1.0 - (j as f32 + 0.5) / rows as f32;
            let mag = w * (0.4 + alt);
            for i in 0..cols {
                let sway = ((self.time * 1.5 + i as f32 * 0.7 + j as f32).sin()) * 3.0;
                let c = vec2(r.x + r.w * (i as f32 + 0.5) / cols as f32, r.y + r.h * (j as f32 + 0.5) / rows as f32 * 0.85 + sway);
                let len = 6.0 + 38.0 * mag.abs();
                let dir = if mag >= 0.0 { 1.0 } else { -1.0 };
                let (a, b) = (c - vec2(len * dir * 0.5, 0.0), c + vec2(len * dir * 0.5, 0.0));
                let col = Color::new(1.0, 1.0, 1.0, 0.25 + 0.55 * mag.abs().min(1.0));
                draw_line(a.x, a.y, b.x, b.y, 2.0, col);
                draw_triangle(b, b - vec2(6.0 * dir, 4.0), b - vec2(6.0 * dir, -4.0), col);
            }
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
                ui.text("Survole une case pour apprendre", r.x + 8.0, r.y + 80.0, 13, DIM);
            }
        }
    }

    fn draw_science(&mut self, ui: &mut Ui, sw: f32, sh: f32) {
        draw_rectangle(PANEL_W, TOP_H, sw - PANEL_W, sh - TOP_H, Color::new(0.02, 0.03, 0.07, 0.86));
        let area = Rect::new(PANEL_W + 30.0, TOP_H + 20.0, sw - PANEL_W - 60.0, sh - TOP_H - 40.0);
        ui.text("Les transformations de la matière", area.x, area.y + 28.0, 28, ACCENT);
        if ui.button(Rect::new(area.x + area.w - 110.0, area.y, 110.0, 36.0), "Fermer", Icon::None, false, true, ACCENT) {
            self.show_science = false;
        }
        let col_w = area.w / 2.0;
        let per_col = RECIPES.len().div_ceil(2);
        for (i, rec) in RECIPES.iter().enumerate() {
            let (col, row) = (i / per_col, i % per_col);
            let x = area.x + col as f32 * col_w;
            let y = area.y + 58.0 + row as f32 * 72.0;
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
        if ui.pressed && !area.contains(ui.mouse) {
            self.show_science = false;
        }
    }
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

#[macroquad::main(window_conf)]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let shot = arg("--demo");

    let font = load_ttf_font_from_bytes(include_bytes!("../assets/font.ttf")).expect("font");
    let mut ui = Ui { font, mouse: Vec2::ZERO, pressed: false };
    let mut renderer = Renderer::new();
    let mut game = Game::new();
    if shot.is_some() {
        let biome = match arg("--biome").as_deref() {
            Some("mer") => Biome::Sea,
            Some("montagne") => Biome::Mountain,
            _ => Biome::Countryside,
        };
        demo_scene(&mut game, biome);
        game.tab = match arg("--tab").as_deref() {
            Some("matter") => Tab::Matter,
            Some("cars") => Tab::Cars,
            Some("tools") => Tab::Tools,
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
            _ => ViewMode::Normal,
        };
        if args.iter().any(|a| a == "--palette") {
            game.views_open = 1.0;
            game.views_pin = true;
        }
        game.show_science = args.iter().any(|a| a == "--science");
        game.tool = if args.iter().any(|a| a == "--loupe") { Tool::Loupe } else { Tool::Block(BlockKind::Brick) };
    }
    let mut frame = 0u32;
    let mut frame_time = 0.0f32;

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
