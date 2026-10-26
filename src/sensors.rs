//! Sensors: a thermometer, a barometer and a hygrometer in one probe, placed in the world with the
//! sensor tool. Each probe keeps a short history and draws its curves in a card at the bottom of
//! the picture, so that one can watch a pot come to the boil and stay at 100 °C, a boiler build
//! up pressure, or the air dry out at noon.

use super::*;
use std::collections::VecDeque;

/// At most this many probes at once.
pub const MAX_SENSORS: usize = 4;
/// A reading every this many simulation steps (half a second of game time).
const SAMPLE_EVERY: u32 = 30;
/// Readings kept: two minutes of game time.
const SAMPLES: usize = 240;
/// A click this close to a probe (cells) removes it.
const PICK_R: f32 = 6.0;
const LETTERS: [&str; MAX_SENSORS] = ["A", "B", "C", "D"];
const COLORS: [Color; MAX_SENSORS] = [
    Color::new(1.0, 0.70, 0.18, 1.0),
    Color::new(0.45, 0.85, 1.0, 1.0),
    Color::new(0.75, 0.55, 1.0, 1.0),
    Color::new(0.45, 0.95, 0.55, 1.0),
];

/// One reading: temperature (°C), absolute pressure (atm), relative humidity (0..1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub temp: f32,
    pub press: f32,
    pub hum: f32,
}

#[derive(Clone)]
pub struct Sensor {
    pub x: i32,
    pub y: i32,
    pub history: VecDeque<Reading>,
}

#[derive(Clone, Default)]
pub struct Sensors {
    pub list: Vec<Sensor>,
    steps: u32,
}

impl Sensors {
    /// Removes the probe under (x, y), or places a new one there (the oldest goes when there are
    /// already `MAX_SENSORS`).
    pub fn toggle(&mut self, world: &World, x: i32, y: i32) {
        if let Some(i) = self.list.iter().position(|s| ((s.x - x) as f32).hypot((s.y - y) as f32) <= PICK_R) {
            self.list.remove(i);
            return;
        }
        if self.list.len() == MAX_SENSORS {
            self.list.remove(0);
        }
        let mut s = Sensor { x, y, history: VecDeque::with_capacity(SAMPLES) };
        s.history.push_back(read(world, x, y));
        self.list.push(s);
    }

    /// Called after every simulation step: takes a reading every `SAMPLE_EVERY` steps.
    pub fn step(&mut self, world: &World) {
        if self.list.is_empty() {
            return;
        }
        self.steps += 1;
        if self.steps < SAMPLE_EVERY {
            return;
        }
        self.steps = 0;
        for s in &mut self.list {
            if s.history.len() == SAMPLES {
                s.history.pop_front();
            }
            s.history.push_back(read(world, s.x, s.y));
        }
    }

    /// Forgets the curves (the world went back to its saved state); the probes stay.
    pub fn clear_history(&mut self, world: &World) {
        for s in &mut self.list {
            s.history.clear();
            s.history.push_back(read(world, s.x, s.y));
        }
    }
}

/// What the probe at (x, y) reads now. In a liquid the pressure is the liquid's (weight of what is
/// above plus the gas pressing on it). Humidity is water: in water (or mud) it is full, in another
/// liquid (lava, oil, molten metal) there is none.
pub fn read(world: &World, x: i32, y: i32) -> Reading {
    let (cx, cy) = (x.clamp(0, W as i32 - 1), y.clamp(0, H as i32 - 1));
    let temp = world.temp[cy as usize * W + cx as usize];
    let liquid = matches!(world.raw(cx, cy).kind(), world::Kind::Liquid);
    let press = if liquid {
        world.liquid_pressure(cx, cy)
    } else if world.pocket_at(cx, cy).is_some() {
        world.gas_pressure(cx, cy)
    } else {
        crate::world::air_pressure_atm(cy)
    };
    let hum = if liquid { if world.raw(cx, cy) == Mat::Water { 1.0 } else { 0.0 } } else { world.flow.moisture_at(cx, cy) };
    Reading { temp, press, hum }
}

/// "1,02" with a French decimal comma.
fn comma(v: f32, decimals: usize) -> String {
    format!("{v:.decimals$}").replace('.', ",")
}

impl Game {
    pub fn sensor_click(&mut self, at: Vec2) {
        self.sensors.toggle(&self.world, at.x.floor() as i32, at.y.floor() as i32);
    }

    /// The probes in the world, and their cards along the bottom of the picture.
    pub fn draw_sensors(&self, ui: &Ui, view: View) {
        for (k, s) in self.sensors.list.iter().enumerate() {
            let p = view.to_screen(vec2(s.x as f32 + 0.5, s.y as f32 + 0.5));
            let col = COLORS[k];
            draw_circle(p.x, p.y, 9.0, Color::new(0.04, 0.05, 0.1, 0.85));
            draw_circle_lines(p.x, p.y, 9.0, 2.0, col);
            draw_circle(p.x, p.y, 2.5, col);
            ui.text(LETTERS[k], p.x + 11.0, p.y - 8.0, 15, col);
        }
        let area = view.visible();
        let (cw, ch, gap) = (236.0, 112.0, 10.0);
        let x0 = area.x + 86.0;
        // Above the arrivals counter in the bottom-left corner.
        let y = (area.y + area.h).min(screen_height()) - ch - 46.0;
        for (k, s) in self.sensors.list.iter().enumerate() {
            let r = Rect::new(x0 + k as f32 * (cw + gap), y, cw, ch);
            if r.x + r.w > area.x + area.w {
                break;
            }
            draw_card(ui, r, k, s);
        }
    }
}

fn draw_card(ui: &Ui, r: Rect, k: usize, s: &Sensor) {
    rrect(r, 10.0, Color::new(0.05, 0.06, 0.11, 0.88));
    let col = COLORS[k];
    let Some(now) = s.history.back().copied() else { return };
    ui.text(LETTERS[k], r.x + 10.0, r.y + 19.0, 17, col);
    ui.text(&format!("{} °C", comma(now.temp, 1)), r.x + 30.0, r.y + 19.0, 15, Color::new(1.0, 0.45, 0.35, 1.0));
    ui.text(&format!("{} atm", comma(now.press, 2)), r.x + 106.0, r.y + 19.0, 15, Color::new(0.45, 0.7, 1.0, 1.0));
    ui.text(&format!("{:.0} %", now.hum * 100.0), r.x + 186.0, r.y + 19.0, 15, Color::new(0.45, 0.9, 0.75, 1.0));
    // Two curves, each scaled to its own range so that a small change still shows.
    let plot = Rect::new(r.x + 10.0, r.y + 28.0, r.w - 20.0, r.h - 52.0);
    draw_rectangle(plot.x, plot.y, plot.w, plot.h, Color::new(1.0, 1.0, 1.0, 0.04));
    let temps: Vec<f32> = s.history.iter().map(|h| h.temp).collect();
    let press: Vec<f32> = s.history.iter().map(|h| h.press).collect();
    let (t0, t1) = curve(plot, &temps, 2.0, Color::new(1.0, 0.45, 0.35, 1.0));
    let (p0, p1) = curve(plot, &press, 0.05, Color::new(0.45, 0.7, 1.0, 1.0));
    ui.text(&format!("{}…{} °C", comma(t0, 0), comma(t1, 0)), r.x + 10.0, r.y + r.h - 8.0, 13, DIM);
    ui.text(&format!("{}…{} atm", comma(p0, 2), comma(p1, 2)), r.x + 120.0, r.y + r.h - 8.0, 13, DIM);
}

/// Draws `values` across `plot`, scaled between their lowest and highest (at least `span` apart).
/// Returns that range.
fn curve(plot: Rect, values: &[f32], span: f32, col: Color) -> (f32, f32) {
    let lo = values.iter().copied().fold(f32::MAX, f32::min);
    let hi = values.iter().copied().fold(f32::MIN, f32::max);
    let mid = 0.5 * (lo + hi);
    let (lo, hi) = if hi - lo < span { (mid - span / 2.0, mid + span / 2.0) } else { (lo, hi) };
    let n = values.len().max(2) as f32 - 1.0;
    let pt = |i: usize, v: f32| vec2(plot.x + plot.w * i as f32 / n, plot.y + plot.h * (1.0 - (v - lo) / (hi - lo)));
    for i in 1..values.len() {
        let (a, b) = (pt(i - 1, values[i - 1]), pt(i, values[i]));
        draw_line(a.x, a.y, b.x, b.y, 1.8, col);
    }
    (lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_probe_in_a_pot_records_the_water_warming_up_but_never_past_100() {
        let mut w = World::new_static();
        for x in 99..=141 {
            w.set(x, 300, Mat::Stone);
        }
        for y in 270..300 {
            w.set(99, y, Mat::Stone);
            w.set(141, y, Mat::Stone);
        }
        for x in 100..141 {
            for y in 285..300 {
                w.set(x, y, Mat::Water);
            }
        }
        let mut s = Sensors::default();
        s.toggle(&w, 120, 298);
        assert_eq!(s.list.len(), 1);
        for _ in 0..7500 {
            // A hot plate under the pot.
            for x in 100..141 {
                w.temp[300 * W + x] = 180.0;
            }
            w.step();
            s.step(&w);
        }
        let h = &s.list[0].history;
        assert_eq!(h.len(), SAMPLES, "the history is full and rolls on");
        let hottest = h.iter().map(|r| r.temp).fold(f32::MIN, f32::max);
        assert!(hottest > 60.0, "the water warmed up: {hottest}");
        assert!(h.iter().all(|r| r.temp <= 101.0), "liquid water never goes past its boiling point");
        assert!(h.iter().all(|r| r.press >= 1.0 && r.press < 1.5), "the pressure at the bottom of the pot is a little above 1 atm");
        assert!(h.iter().any(|r| r.press > 1.0), "the weight of the water above shows");
        assert!(h.iter().filter(|r| r.hum == 1.0).count() > SAMPLES / 2, "in water the humidity is full");
        // Clicking the probe again removes it.
        s.toggle(&w, 121, 299);
        assert!(s.list.is_empty());
    }

    #[test]
    fn lava_and_oil_hold_no_water() {
        let mut w = World::new_static();
        w.set(100, 100, Mat::Lava);
        w.set(110, 100, Mat::Oil);
        w.set(120, 100, Mat::Water);
        assert_eq!(read(&w, 100, 100).hum, 0.0, "lava is dry");
        assert_eq!(read(&w, 110, 100).hum, 0.0, "oil is dry");
        assert_eq!(read(&w, 120, 100).hum, 1.0, "water is full");
        let field = w.humidity_field();
        assert_eq!(w.humidity_color(&field, 100, 100), [58, 62, 74, 255], "the humidity view shows lava as a dry solid");
        assert_ne!(w.humidity_color(&field, 120, 100), [58, 62, 74, 255]);
    }

    #[test]
    fn only_four_probes_the_oldest_goes() {
        let w = World::new_static();
        let mut s = Sensors::default();
        for k in 0..5 {
            s.toggle(&w, 100 + 50 * k, 100);
        }
        assert_eq!(s.list.len(), MAX_SENSORS);
        assert_eq!(s.list[0].x, 150);
        let (high, low) = (read(&w, 100, 100).press, read(&w, 100, 355).press);
        assert!((low - 1.0).abs() < 0.01, "open air on the valley floor is at 1 atm: {low}");
        assert!(high > 0.2 && high < 0.6, "the air thins out with altitude, 8 km up: {high}");
    }
}
