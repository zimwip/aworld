//! The frame counter in the bottom right corner (F3 shows or hides it): images a second (the
//! picture, at the pace of the screen), steps a second (the world, 60 a second at x1 whatever the
//! screen), and how long one step takes, all averaged over half a second so they can be read.

use crate::gfx::*;

use crate::ui::{rrect, Ui, DIM, GREEN, RED};

/// How often the figures are refreshed, in seconds.
const EVERY: f64 = 0.5;

pub struct Fps {
    pub shown: bool,
    since: f64,
    frames: u32,
    step_time: f64,
    steps: u32,
    /// The last figures: images a second, steps a second, and milliseconds a step (None while
    /// the world is paused).
    fps: f32,
    pps: f32,
    step_ms: Option<f32>,
}

impl Fps {
    pub fn new() -> Self {
        Fps { shown: true, since: 0.0, frames: 0, step_time: 0.0, steps: 0, fps: 0.0, pps: 0.0, step_ms: None }
    }

    /// The time spent on `steps` steps of the simulation this frame.
    pub fn steps(&mut self, seconds: f64, steps: usize) {
        self.step_time += seconds;
        self.steps += steps as u32;
    }

    /// The steps counted since the last call, and their time (the simulation thread hands its
    /// count to the screen's counter this way).
    pub fn take_steps(&mut self) -> (f64, usize) {
        let taken = (self.step_time, self.steps as usize);
        self.step_time = 0.0;
        self.steps = 0;
        taken
    }

    /// Once a frame.
    pub fn frame(&mut self) {
        self.frames += 1;
        let now = get_time();
        // The clock starts with the first frame (a game made before the window, as in the tests,
        // has no clock to read yet).
        if self.since == 0.0 {
            self.since = now;
        }
        let span = now - self.since;
        if span >= EVERY {
            self.fps = (self.frames as f64 / span) as f32;
            self.pps = (self.steps as f64 / span) as f32;
            self.step_ms = (self.steps > 0).then(|| (self.step_time * 1000.0 / self.steps as f64) as f32);
            self.since = now;
            self.frames = 0;
            self.step_time = 0.0;
            self.steps = 0;
        }
    }

    pub fn draw(&self, ui: &Ui, sw: f32, sh: f32) {
        if !self.shown {
            return;
        }
        let fps = format!("{:.0} FPS", self.fps);
        let step = match self.step_ms {
            Some(ms) => format!("{:.0} PPS · {ms:.1} ms/pas", self.pps),
            None => "monde en pause".to_string(),
        };
        let size = 15;
        let (wf, ws) = (ui.text_w(&fps, size), ui.text_w(&step, size));
        let w = wf + ws + 36.0;
        let r = Rect::new(sw - w - 8.0, sh - 30.0, w, 22.0);
        rrect(r, 8.0, Color::new(0.05, 0.06, 0.1, 0.7));
        let colour = if self.fps >= 55.0 {
            GREEN
        } else if self.fps >= 30.0 {
            Color::new(1.0, 0.78, 0.25, 1.0)
        } else {
            RED
        };
        ui.text(&fps, r.x + 10.0, r.y + 16.0, size, colour);
        ui.text(&step, r.x + wf + 26.0, r.y + 16.0, size, DIM);
    }
}
