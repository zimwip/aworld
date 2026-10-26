//! The world runs in a thread of its own on the desktop, and the screen draws at its own pace.
//!
//! The simulation thread holds the real world (a whole `Game`, of which it uses the part that
//! lives: the cells, the blocks, the cars, the visitors...). After each update it copies that part
//! into a spare `Game`, the picture, and hands it over; the screen swaps the picture into its own
//! `Game` and gives the old one back to be filled again. So the screen always has a whole,
//! steady world to draw, 60 or 144 times a second, however long a step takes: only the steps a
//! second (PPS) drop when the world is heavy, not the images a second (FPS).
//!
//! The screen's own part (the camera, the menus, the tool, the sound, the message at the bottom)
//! never leaves the main thread. When a click, a key or the wheel may change the world, the screen
//! borrows the real world for that frame: the simulation thread stops after the step it is on and
//! sends it over, the frame runs its tools, buttons and drawing on it as on one thread, and the
//! world goes back with the edits. The next picture shows them.
//!
//! The web build has no threads: it keeps the one loop of `main`, as do `--demo` and the tests.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Instant;

use macroquad::prelude::*;

use crate::sound::{Ev, Sounds};
use crate::ui::Ui;
use crate::world::World;
use crate::{ctrl_down, Game};

/// The part of a `Game` that lives in the simulation thread and that the screen reads: copied into
/// each picture, and swapped with the screen's when the world is borrowed.
macro_rules! shown {
    ($m:ident, $a:expr, $b:expr) => {
        $m!($a, $b; blocks, cables, members, springs, mech, cars, debris, dynamite, sensors, trucks, kids, planes, whales,
            wolves, fireworks, fireworks_told, booms, bolts, thunder, shake, time, arrived, lost, census, speed, selected_car,
            running, shovel_age, mix)
    };
}

/// The rest of the living part: only the simulation reads it, so it is not copied into the
/// pictures, but it goes to the screen with the world when it is borrowed.
macro_rules! inner {
    ($m:ident, $a:expr, $b:expr) => {
        $m!($a, $b; phys, beam_broke, broken_members, heat_cells, frame_ema, slow_frames, fast_frames, lod_lock, sim_acc)
    };
}

macro_rules! swap {
    ($a:expr, $b:expr; $($f:ident),*) => {
        $(std::mem::swap(&mut $a.$f, &mut $b.$f);)*
    };
}

macro_rules! copy {
    ($from:expr, $to:expr; $($f:ident),*) => {
        $($to.$f.clone_from(&$from.$f);)*
    };
}

/// Swaps the living part of two games (the world goes from one thread's game to the other's).
fn swap_living(a: &mut Game, b: &mut Game) {
    std::mem::swap(&mut a.world, &mut b.world);
    shown!(swap, a, b);
    inner!(swap, a, b);
}

/// Makes `to` a picture of `from` (see the top of the file).
fn copy_picture(from: &Game, to: &mut Game) {
    to.world.copy_picture(&from.world);
    shown!(copy, from, to);
    // The physics stays with the simulation: the stress view gets the loads on the welds.
    to.stress_seen = from.phys.as_ref().filter(|_| from.mode == crate::ViewMode::Stress).map(|p| p.stress(&from.blocks));
}

/// What the screen owns but the simulation reads: the screen sets it, the world gets it when it is
/// given back.
fn copy_settings(from: &Game, to: &mut Game) {
    to.tool = from.tool;
    to.tab = from.tab;
    to.exit = from.exit;
    to.mode = from.mode;
}

/// What the simulation thread has to tell the screen besides the world: the message at the
/// bottom, the sounds to play, the steps done (for the counter).
struct News {
    toast: Option<(String, f32)>,
    sounds: Vec<Ev>,
    steps: (f64, usize),
}

impl News {
    fn take(g: &mut Game) -> News {
        News { toast: g.toast.take(), sounds: g.sounds.take_events(), steps: g.fps.take_steps() }
    }

    fn tell(self, g: &mut Game) {
        if let Some(t) = self.toast {
            g.toast = Some(t);
        }
        for e in self.sounds {
            g.sounds.emit(e);
        }
        g.fps.steps(self.steps.0, self.steps.1);
    }
}

enum ToSim {
    /// The screen shows a new image: time for an update of the world.
    Tick,
    /// The screen needs the real world for this frame.
    Lend,
    /// The world, back from the screen; and an old picture to fill again.
    Back(Box<Game>),
    Recycle(Box<Game>),
}

enum ToScreen {
    /// A picture, numbered by the lends it follows.
    Picture(Box<Game>, News, u64),
    Lent(Box<Game>, News),
}

/// The screen's side.
pub struct SimThread {
    to: Sender<ToSim>,
    from: Receiver<ToScreen>,
    /// Lends so far, and the number of the picture on the screen: after a lend, the screen waits
    /// for a picture that shows its edits rather than going back to an older one.
    lends: u64,
    shown: u64,
}

impl SimThread {
    /// Sends the living part of `game` to a new simulation thread; `game` keeps a picture of it.
    pub fn start(game: &mut Game) -> SimThread {
        let mut sim = Box::new(blank());
        swap_living(game, &mut sim);
        copy_settings(game, &mut sim);
        copy_picture(&sim, game);
        sim.sounds = Sounds::recorder();
        let (to, rx) = channel();
        let (tx, from) = channel();
        let spare = Box::new(blank());
        std::thread::Builder::new()
            .name("simulation".into())
            .spawn(move || Worker { game: Some(sim), spare: Some(spare), rx, tx, lends: 0, dirty: true, ticked: false, last: Instant::now(), busy: 0.016, lent_for: 0.0 }.run())
            .expect("simulation thread");
        SimThread { to, from, lends: 0, shown: 0 }
    }

    /// At the start of a frame: the latest picture into `game`, or, when `borrow`, the real world
    /// for this frame (give it back with `give_back` once the frame is drawn).
    pub fn frame(&mut self, game: &mut Game, borrow: bool) -> Option<Box<Game>> {
        if borrow {
            let _ = self.to.send(ToSim::Lend);
            loop {
                match self.from.recv() {
                    Ok(ToScreen::Picture(pic, news, n)) => self.show(game, pic, news, n),
                    Ok(ToScreen::Lent(mut sim, news)) => {
                        news.tell(game);
                        swap_living(game, &mut sim);
                        return Some(sim);
                    }
                    Err(_) => panic!("the simulation thread stopped"),
                }
            }
        }
        let _ = self.to.send(ToSim::Tick);
        loop {
            let next = if self.shown < self.lends { self.from.recv().ok() } else { self.from.try_recv().ok() };
            match next {
                Some(ToScreen::Picture(pic, news, n)) => self.show(game, pic, news, n),
                Some(ToScreen::Lent(..)) => unreachable!("lent without being asked"),
                None => return None,
            }
        }
    }

    /// The world goes back to its thread with this frame's edits.
    pub fn give_back(&mut self, game: &mut Game, mut sim: Box<Game>) {
        swap_living(game, &mut sim);
        copy_settings(game, &mut sim);
        self.lends += 1;
        let _ = self.to.send(ToSim::Back(sim));
        let _ = self.to.send(ToSim::Tick);
    }

    fn show(&mut self, game: &mut Game, mut pic: Box<Game>, news: News, n: u64) {
        std::mem::swap(&mut game.world, &mut pic.world);
        shown!(swap, game, pic);
        std::mem::swap(&mut game.stress_seen, &mut pic.stress_seen);
        news.tell(game);
        self.shown = n;
        let _ = self.to.send(ToSim::Recycle(pic));
    }
}

/// Could this frame's input change the world? A click or a held button (but on the camera's
/// Ctrl), a key, the wheel: the frame then borrows the real world. Moving the mouse over the world
/// or the menus does not.
pub fn input_may_edit(game: &Game, ui: &Ui) -> bool {
    let buttons = [MouseButton::Left, MouseButton::Right, MouseButton::Middle];
    let mouse = buttons.iter().any(|&b| is_mouse_button_down(b) || is_mouse_button_pressed(b) || is_mouse_button_released(b));
    let wheel = mouse_wheel() != (0.0, 0.0);
    // Ctrl + drag and Ctrl + wheel only move the camera (see `handle_camera`).
    let camera = (ctrl_down() || game.camera.dragging()) && !ui.over_ui();
    ((mouse || wheel) && !camera) || !get_keys_pressed().is_empty()
}

/// A game with an empty world, to hold pictures (and the world while it is with the screen).
fn blank() -> Game {
    Game::with_world(World::new(), (0, 0), (0, 0))
}

/// The simulation thread's side.
struct Worker {
    game: Option<Box<Game>>,
    spare: Option<Box<Game>>,
    rx: Receiver<ToSim>,
    tx: Sender<ToScreen>,
    lends: u64,
    /// The world changed since the last picture; an image was asked for since the last update.
    dirty: bool,
    ticked: bool,
    /// When the last update began, and how long its work took.
    last: Instant,
    busy: f32,
    /// How long the world was lent during this update.
    lent_for: f32,
}

impl Worker {
    fn run(mut self) {
        loop {
            // Nothing to do until the screen asks for something.
            if !self.ticked {
                match self.rx.recv() {
                    Ok(m) => self.handle(m),
                    Err(_) => return,
                }
            }
            if !self.poll() || self.game.is_none() {
                return;
            }
            if self.ticked {
                self.ticked = false;
                self.update();
                self.dirty = true;
            }
            if self.dirty {
                if let Some(mut pic) = self.spare.take() {
                    let game = self.game.as_mut().expect("the world");
                    copy_picture(game, &mut pic);
                    let news = News::take(game);
                    self.dirty = false;
                    if self.tx.send(ToScreen::Picture(pic, news, self.lends)).is_err() {
                        return;
                    }
                }
            }
        }
    }

    /// One update of the world, as long as the time since the last one (as the screen's frame
    /// did on one thread), its steps stopping for a lend when the screen asks. The world is
    /// simplified when the updates themselves are slow, not the screen.
    fn update(&mut self) {
        let dt = self.last.elapsed().as_secs_f32().min(0.05);
        self.last = Instant::now();
        let pace = self.busy.min(0.05);
        let start = Instant::now();
        self.lent_for = 0.0;
        self.update_steps(dt, pace);
        // The time the world spent with the screen is not work.
        self.busy = (start.elapsed().as_secs_f32() - self.lent_for).max(0.0);
    }

    fn update_steps(&mut self, dt: f32, pace: f32) {
        let Some(steps) = self.game.as_mut().expect("the world").update_world(dt, pace) else { return };
        let mut seconds = 0.0;
        for _ in 0..steps {
            if !self.poll() || self.game.is_none() {
                return;
            }
            let start = Instant::now();
            self.game.as_mut().expect("the world").sim_step();
            seconds += start.elapsed().as_secs_f64();
        }
        self.game.as_mut().expect("the world").after_steps(dt, steps, seconds);
    }

    /// Reads what the screen sent; false once it is gone (the game is closing).
    fn poll(&mut self) -> bool {
        loop {
            match self.rx.try_recv() {
                Ok(m) => self.handle(m),
                Err(std::sync::mpsc::TryRecvError::Empty) => return true,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return false,
            }
        }
    }

    fn handle(&mut self, m: ToSim) {
        match m {
            ToSim::Tick => self.ticked = true,
            ToSim::Recycle(pic) => self.spare = Some(pic),
            ToSim::Back(_) => unreachable!("given back without being lent"),
            ToSim::Lend => {
                let since = Instant::now();
                let mut game = self.game.take().expect("the world");
                let news = News::take(&mut game);
                if self.tx.send(ToScreen::Lent(game, news)).is_err() {
                    return;
                }
                // Wait for it (the screen may tick and recycle meanwhile).
                while self.game.is_none() {
                    match self.rx.recv() {
                        Ok(ToSim::Back(game)) => self.game = Some(game),
                        Ok(m) => self.handle(m),
                        Err(_) => return,
                    }
                }
                self.lends += 1;
                self.dirty = true;
                self.lent_for += since.elapsed().as_secs_f32();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_world_runs_in_its_thread_and_a_lend_brings_the_edits_to_it() {
        let mut g = Game::new();
        g.set_running(true);
        let start = g.world.clock;
        let mut sim = SimThread::start(&mut g);
        // The screen only shows pictures, and the world goes on.
        for _ in 0..200 {
            assert!(sim.frame(&mut g, false).is_none());
            std::thread::sleep(std::time::Duration::from_millis(5));
            if g.world.clock > start + 3 {
                break;
            }
        }
        assert!(g.world.clock > start + 3, "the world did not run: {} steps", g.world.clock - start);
        assert!(g.phys.is_none(), "the physics stays in the simulation thread");
        // A frame borrows the real world (with its physics), edits it and gives it back.
        let lent = sim.frame(&mut g, true).expect("lent");
        assert!(g.phys.is_some());
        let clock = g.world.clock;
        g.world.wind_max = 0.123;
        g.speed = 1;
        sim.give_back(&mut g, lent);
        // The next frame waits for a picture made after the lend: it shows the edits.
        sim.frame(&mut g, false);
        assert_eq!(g.world.wind_max, 0.123);
        assert_eq!(g.speed, 1);
        assert!(g.world.clock >= clock);
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    /// `cargo test --release copy_cost -- --ignored --nocapture`: what a picture costs the simulation.
    #[test]
    #[ignore]
    fn copy_cost() {
        let mut g = Game::new();
        g.set_running(true);
        for _ in 0..30 {
            g.sim_step();
        }
        let mut pic = blank();
        let t = Instant::now();
        let n = 50;
        for _ in 0..n {
            copy_picture(&g, &mut pic);
        }
        let copy = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        let t = Instant::now();
        for _ in 0..n {
            g.sim_step();
        }
        let step = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        println!("picture {copy:.2} ms, step {step:.2} ms");
    }
}
