//! The scenes and scripted moments of the screenshots (`--demo`): the scenes are built at the
//! start (`setup`), and the moments (an explosion, a bolt, a fire engine) come at fixed frames
//! before the shot (`at_frame`).

use super::*;

/// The value after `name` on the command line.
fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// Builds the screenshot's scene, with its tab, view and visitors, from the command line.
pub fn setup(game: &mut Game, args: &[String]) {
    let arg = |name: &str| arg(args, name);
    // Screenshots stay as they were: no frame counter in them.
    game.fps.shown = args.iter().any(|a| a == "--fps");
    game.metrics.shown = args.iter().any(|a| a == "--metrics");
    game.panel.pin = !std::env::args().any(|a| a == "--hide-panel");
    let biome = match arg("--biome").as_deref() {
        Some("mer") => Biome::Sea,
        Some("montagne") => Biome::Mountain,
        _ => Biome::Countryside,
    };
    demo_scene(game, biome);
    game.tab = match arg("--tab").as_deref() {
        Some("matter") => Tab::Matter,
        Some("cars") => Tab::Cars,
        Some("mech") => Tab::Mech,
        Some("planet") => Tab::Planet,
        _ => Tab::Blocks,
    };
    // `--sow`: seeds sown along the ground, all across the world (before `--lawn`'s steps).
    if args.iter().any(|a| a == "--sow") {
        for x in (0..W() as i32).step_by(7) {
            let y = (0..H() as i32).find(|&y| matches!(game.world.raw(x, y), Mat::Dirt | Mat::Grass)).unwrap_or(0);
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
        demo_pipes(game);
        game.set_running(true);
        game.tab = Tab::Fluid;
    }
    if args.iter().any(|a| a == "--select") {
        game.selection = game.blocks.first().map(|b| select::Sel::Block(b.uid));
    }
    if args.iter().any(|a| a == "--bridge") {
        demo_bridge(game);
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
        demo_mech(game);
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
        demo_tunnel(game);
    }
    if args.iter().any(|a| a == "--balance") {
        demo_balance(game);
    }
    if args.iter().any(|a| a == "--dam") {
        demo_dam(game);
    }
    if args.iter().any(|a| a == "--magnet") {
        demo_magnet(game);
    }
    if args.iter().any(|a| a == "--crater") {
        demo_crater(game, args.iter().any(|a| a == "--dynamite"));
    }
    if args.iter().any(|a| a == "--rain") {
        demo_rain(game);
    }
    if args.iter().any(|a| a == "--sticks") {
        demo_sticks(game);
    }
    if args.iter().any(|a| a == "--quarry") {
        demo_quarry(game);
    }
    if args.iter().any(|a| a == "--topple") {
        demo_topple(game);
    }
    if args.iter().any(|a| a == "--siphon") {
        demo_siphon(game);
    }
    if args.iter().any(|a| a == "--logs") {
        demo_logs(game);
    }
    if args.iter().any(|a| a == "--soil") {
        demo_soil(game);
    }
    if args.iter().any(|a| a == "--lava") {
        demo_lava(game);
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

/// What the command line asks for at frame `frame` of the screenshot (before the frame is drawn).
pub fn at_frame(game: &mut Game, args: &[String], frame: u32) {
    let arg = |name: &str| arg(args, name);
    // `--boom`: an explosion 25 cells under the water surface at x (default 360), shortly before the shot.
    if frame == 357 && args.iter().any(|a| a == "--boom") {
        let x = arg("--boom").and_then(|v| v.parse().ok()).unwrap_or(360);
        if let Some(top) = (0..H() as i32).find(|&y| game.world.at(x, y) == Mat::Water) {
            game.world.explode(x, top + 25, 18);
        }
    }

    // `--fireworks`: three salvos, bursting just before the shot. `--whale`: a whale that is
    // blowing at the shot. `--firetruck x`: a fire engine coming to a fire at x.
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
                t.y = (0..H() as i32).find(|&y| game.world.hard_for_car(t.x as i32, y)).unwrap_or(400) as f32;
            }
        }
    }
    // `--truck-boom n`: the fire engine is set off n frames before the shot.
    if let Some(n) = arg("--truck-boom").and_then(|v| v.parse::<u32>().ok()) {
        if frame + n == 360 {
            game.blow_up_truck();
        }
    }
    // `--crater n`: the buried powder is lit n frames before the shot.
    if let Some(n) = arg("--crater").and_then(|v| v.parse::<u32>().ok()) {
        if frame + n == 360 {
            game.world.set(480, 359, Mat::Fire);
            game.world.set(480, 360, Mat::Fire);
            game.world.set(480, 365, Mat::Fire);
        }
    }
    // `--wreck`: the first car blows up shortly before the shot, so its pieces are in the air.
    if frame == 300 && args.iter().any(|a| a == "--wreck") {
        game.launch_car();
    }
    if frame == 345 && args.iter().any(|a| a == "--wreck") {
        if let Some(c) = game.cars.last_mut() {
            c.fate = Fate::Exploded;
        }
    }
    // `--anvil`: a plume of warm damp air off the plain, run until it has built a storm cloud
    // whose top spreads out under the tropopause and is dragged downwind into an anvil.
    if args.iter().any(|a| a == "--anvil") {
        // First a long run to build the cloud, then the plume keeps feeding it until the shot.
        let (from, to) = if frame == 1 { (0, 2600u32) } else { (2600 + frame * 4, 2604 + frame * 4) };
        for step in from..to {
            for k in 0..8u32 {
                let x = 600 + ((step * 7919 + k * 104729) % 120) as i32;
                let ground = (0..H() as i32).find(|&y| !matches!(game.world.at(x, y), Mat::Empty | Mat::Steam | Mat::Cloud | Mat::Smoke)).unwrap_or(H() as i32);
                let y = ground - 2 - ((step + k * 13) % 6) as i32;
                if game.world.at(x, y) == Mat::Empty {
                    game.world.set(x, y, Mat::Steam);
                    game.world.temp[y as usize * world::W() + x as usize] = 32.0;
                }
            }
            if frame == 1 {
                game.world.step();
            }
        }
    }
    // `--storm`: a big storm cloud, charged (positive top, negative base), over the valley.
    // `--lightning x`: a bolt from the sky at x, just before the shot.
    if frame == 345 && args.iter().any(|a| a == "--storm") {
        for x in 280..680 {
            let top = 70 + ((x as f32 * 0.05).sin() * 8.0) as i32;
            for y in top..200 {
                game.world.set(x, y, Mat::Cloud);
            }
        }
        for qy in 0..world::QH() {
            for qx in 0..world::QW() {
                let (x, y) = ((qx * world::QS()) as f32, (qy * world::QS()) as f32);
                let inside = (280.0..680.0).contains(&x) && (70.0..200.0).contains(&y);
                if inside {
                    let u = ((x - 480.0) / 200.0).powi(2);
                    game.world.storm.charge[qy * world::QW() + qx] = (if y < 120.0 { 0.8 } else if y > 150.0 { -0.95 } else { 0.0 }) * (1.0 - u);
                }
            }
        }
        // The ground under it has had time to take the opposite charge.
        game.world.settle_ground_charge(150);
    }
    if frame == 356 {
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
    if frame == 330 && args.iter().any(|a| a == "--clouds") {
        for (x, y, r) in [(150, 150, 12), (175, 158, 10), (420, 130, 14), (450, 138, 10), (700, 160, 12), (735, 152, 13), (870, 140, 11)] {
            game.world.paint(x, y, r, Mat::Cloud);
        }
    }
    // `--pour x`: a heap of sand and a ball of water in the air over x, shortly before the shot.
    if frame == 320 {
        if let Some(x) = arg("--pour").and_then(|v| v.parse::<i32>().ok()) {
            let y = ground_y(&game.world, x) as i32;
            game.world.paint(x - 14, y - 40, 8, Mat::Sand);
            game.world.paint(x + 14, y - 46, 9, Mat::Water);
        }
    }
    // `--deluge`: a great mass of water let fall over the valley (as the bench's scene
    // « déluge »), well before the shot.
    if frame == 120 && args.iter().any(|a| a == "--deluge") {
        for y in 40..230 {
            for x in 150..760 {
                if game.world.at(x, y) == Mat::Empty {
                    game.world.set(x, y, Mat::Water);
                }
            }
        }
    }
    // `--steam x`: a layer of steam on the ground at x, under a layer of water.
    if frame == 300 {
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
    if frame == 352 {
        if let Some(n) = arg("--sim-steps").and_then(|v| v.parse::<usize>().ok()) {
            game.step_by_step(n);
        }
    }
    if frame == 245 && args.iter().any(|a| a == "--bridge") {
        game.launch_car();
    }
}

fn ground_y(world: &World, x: i32) -> i32 {
    (0..H() as i32).find(|&y| world.raw(x, y).is_hard()).unwrap_or(H() as i32 - 1)
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
    for x in 0..W() as i32 {
        for y in ground..H() as i32 {
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
    rect(g, 0, 470, W() as i32 - 1, H() as i32 - 1, Mat::Stone);
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
    for x in 0..W() as i32 {
        for y in floor..H() as i32 {
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

/// `--balance`: free bodies on bedrock, for the stress view: a block of stone centred on a
/// pedestal (it holds), a plank resting off-centre on a post (it holds, just), and a ball of
/// stone on a long arm whose foot stands in a pit of sand: its centre of mass is far beyond its
/// foot, and it tips over.
fn demo_balance(g: &mut Game) {
    g.empty_world();
    let floor = 470;
    let rect = |g: &mut Game, x0: i32, y0: i32, x1: i32, y1: i32, m: Mat| {
        for y in y0..=y1 {
            for x in x0..=x1 {
                g.world.set(x, y, m);
            }
        }
    };
    rect(g, 0, floor, W() as i32 - 1, H() as i32 - 1, Mat::Stone);
    rect(g, 600, floor, 700, floor + 10, Mat::Sand);
    rect(g, 60, 400, 120, floor - 1, Mat::Glass);
    rect(g, 70, 370, 110, 399, Mat::Stone);
    rect(g, 800, 380, 806, floor - 1, Mat::Glass);
    rect(g, 770, 374, 830, 379, Mat::Wood);
    g.world.paint(330, 230, 45, Mat::Stone);
    g.world.line((370, 240), (650, 470), 8, Mat::Stone);
    for i in 0..W() * H() {
        g.world.temp[i] = 15.0;
    }
    g.set_running(true);
}

/// `--dam`: two dams on bedrock, liquids by their physics. On the left a thick wall of rock holds
/// back a deep lake; on the right a pane of glass holds a shallower one, until the water pushes
/// it over. Best seen in the stress view (`--view efforts`).
fn demo_dam(g: &mut Game) {
    g.empty_world();
    let floor = 440;
    for x in 0..W() as i32 {
        for y in floor..H() as i32 {
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
    for i in 0..W() * H() {
        g.world.temp[i] = 15.0;
    }
    g.set_running(true);
}

/// A few bricks on flat rock, the brick tool in hand: with `--mouse` near them, the ghost is pulled
/// into line and the guides show (`--magnet`).
fn demo_magnet(g: &mut Game) {
    g.empty_world();
    let floor = 420;
    for x in 0..W() as i32 {
        for y in floor..H() as i32 {
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
    for x in 0..W() as i32 {
        g.world.set(x, 330, Mat::Grass);
        for y in 331..H() as i32 {
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
    for x in 0..W() as i32 {
        for y in 300..H() as i32 {
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

/// A bank of cloud raining over the meadow, and a plume of steam: `--rain`, for the pictures of
/// the clouds and the drops.
fn demo_rain(g: &mut Game) {
    for (k, x) in (150..820).step_by(13).enumerate() {
        g.world.paint(x, 120 + (k as i32 * 7) % 15, 15 + (k as i32 * 5) % 6, Mat::Cloud);
    }
    for k in 0..6 {
        g.world.paint(560 + k * 4, 330 - k * 22, 8 + k * 2, Mat::Steam);
    }
    for i in 0..2400u32 {
        let h = i.wrapping_mul(2654435761);
        let (x, y) = (160 + (h % 640) as i32, 160 + ((h >> 12) % 200) as i32);
        if g.world.raw(x, y) == Mat::Empty {
            g.world.set(x, y, Mat::Water);
        }
    }
    g.set_running(true);
}

/// A meadow over rock with a big charge of powder buried in it: `--crater` lights it. With
/// `dynamite`, the charge is a stick of dynamite packed in charcoal (it leaves diamonds).
fn demo_crater(g: &mut Game, dynamite: bool) {
    g.empty_world();
    let ground = 330;
    for x in 0..W() as i32 {
        let top = ground + ((x as f32 * 0.02).sin() * 4.0) as i32;
        g.world.set(x, top, Mat::Grass);
        for y in top + 1..H() as i32 {
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
    for x in 0..W() as i32 {
        g.world.set(x, 330, Mat::Grass);
        for y in 331..H() as i32 {
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

/// `--demo --soil`: a bank of wet earth (sand, humus and water) with grass on top, right against a
/// pool of fresh lava in a basin of rock. With `--advance n` the earth boils dry, its humus burns
/// off, the sand left slumps into the lava, fuses into glass and melts into it.
fn demo_soil(g: &mut Game) {
    g.empty_world();
    let floor = 360;
    for x in 0..W() as i32 {
        for y in floor..H() as i32 {
            g.world.set(x, y, Mat::Stone);
        }
    }
    for x in 360..560 {
        let top = if x < 470 { floor - 70 } else { floor - 70 + (x - 470) / 3 };
        for y in top..floor {
            g.world.set(x, y, if y == top { Mat::Grass } else { Mat::Dirt });
            g.world.moisture[y as usize * W() + x as usize] = 150;
        }
    }
    // The basin: a thick wall of terracotta (it does not melt), the lava poured in against the
    // earth up to the top of the bank.
    for y in floor - 80..floor {
        for x in 680..700 {
            g.world.set(x, y, Mat::Terracotta);
        }
    }
    for x in 480..680 {
        for y in floor - 66..floor {
            if g.world.raw(x, y) == Mat::Empty {
                g.world.set(x, y, Mat::Lava);
                g.world.temp[y as usize * W() + x as usize] = 1150.0;
            }
        }
    }
}

/// `--demo --topple`: a long stroke of ice painted from the ground up into the sky, leaning far
/// out, as a player draws it.
fn demo_topple(g: &mut Game) {
    g.empty_world();
    // Fields of earth over the rock, and a pond in a hollow.
    for x in 0..W() as i32 {
        let top = 330 + ((x as f32 * 0.013).sin() * 8.0) as i32;
        let bed = if (380..540).contains(&x) { top + (40.0 * (std::f32::consts::PI * (x - 380) as f32 / 160.0).sin()) as i32 } else { top };
        for y in top..H() as i32 {
            let m = if y > 380 + (x * 7 % 5) { Mat::Stone } else if y < bed { Mat::Water } else if y == top { Mat::Grass } else { Mat::Dirt };
            g.world.set(x, y, m);
        }
    }
    let ground = |w: &World, x: i32| (0..H() as i32).find(|&y| w.at(x, y).is_hard()).unwrap_or(H() as i32);
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
    for x in 0..W() as i32 {
        if (430..530).contains(&x) {
            continue;
        }
        for y in 330..H() as i32 {
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
    for x in 0..W() as i32 {
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
