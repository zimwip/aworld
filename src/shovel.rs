//! The shovel: scoops up a heap of one material (ash from a fire, sand, water, iron...) and pours it
//! out somewhere else, keeping its temperature.

use super::*;

/// What is on the shovel.
#[derive(Clone, Copy, Debug)]
pub struct Load {
    pub mat: Mat,
    pub count: u32,
    pub temp: f32,
    /// What a full scoop would have held (the bubble's size).
    pub cap: u32,
}

fn scoopable(m: Mat) -> bool {
    !matches!(m.kind(), world::Kind::Empty | world::Kind::Gas | world::Kind::Flame)
}

impl Game {
    /// How much one scoop takes: more with a bigger brush.
    fn shovel_capacity(&self) -> u32 {
        (40 + self.brush.max(0) as u32 * 5).min(220)
    }

    /// Scoops the heap of the material under (or next to) `at`.
    pub fn shovel_scoop(&mut self, at: Vec2) {
        let (cx, cy) = (at.x.floor() as i32, at.y.floor() as i32);
        // The material under the cursor, else the nearest one within reach.
        let mut start: Option<(f32, i32, i32)> = None;
        for dy in -4..=4 {
            for dx in -4..=4 {
                let (x, y) = (cx + dx, cy + dy);
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                if d <= 4.0 && self.world.block_at_cell(x, y).is_none() && scoopable(self.world.raw(x, y)) && start.map_or(true, |s| d < s.0) {
                    start = Some((d, x, y));
                }
            }
        }
        let Some((_, sx, sy)) = start else {
            self.say("Rien à prendre ici");
            return;
        };
        let mat = self.world.raw(sx, sy);
        let reach = 5.0 + self.brush as f32 * 0.6;
        let cap = self.shovel_capacity() as usize;
        // The connected heap of that material, nearest first.
        let mut seen = vec![(sx, sy)];
        let mut head = 0;
        while head < seen.len() && seen.len() < cap * 3 {
            let (x, y) = seen[head];
            head += 1;
            for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                let d = (((nx - cx) * (nx - cx) + (ny - cy) * (ny - cy)) as f32).sqrt();
                if d <= reach && self.world.raw(nx, ny) == mat && self.world.block_at_cell(nx, ny).is_none() && !seen.contains(&(nx, ny)) {
                    seen.push((nx, ny));
                }
            }
        }
        seen.sort_by(|a, b| {
            let da = (a.0 - cx).pow(2) + (a.1 - cy).pow(2);
            let db = (b.0 - cx).pow(2) + (b.1 - cy).pow(2);
            da.cmp(&db)
        });
        seen.truncate(cap);
        let mut heat = 0.0;
        for &(x, y) in &seen {
            heat += self.world.temp[y as usize * world::W() + x as usize];
            self.world.set(x, y, Mat::Empty);
        }
        let count = seen.len() as u32;
        self.shovel = Some(Load { mat, count, temp: heat / count.max(1) as f32, cap: cap as u32 });
        self.shovel_age = 0.0;
        self.say(format!("Pelle : {count} de {} (garde le clic : plus longtemps, plus gros morceau ; relâche pour verser)", science::info(mat).name.to_lowercase()));
    }

    /// Pours up to `max` cells of the load near `at`. Returns how many came out.
    pub fn shovel_pour(&mut self, at: Vec2, max: u32) -> u32 {
        let Some(mut load) = self.shovel else { return 0 };
        let (cx, cy) = (at.x.floor() as i32, at.y.floor() as i32);
        let mut poured = 0;
        for _ in 0..max.min(load.count) {
            let mut best: Option<(i32, i32, i32)> = None;
            for dy in -8..=8 {
                for dx in -8..=8 {
                    let (x, y) = (cx + dx, cy + dy);
                    let d = dx * dx + dy * dy;
                    if d <= 64 && self.world.raw(x, y) == Mat::Empty && self.world.block_at_cell(x, y).is_none() && best.map_or(true, |b| d < b.2) {
                        best = Some((x, y, d));
                    }
                }
            }
            let Some((x, y, _)) = best else { break };
            self.world.set(x, y, load.mat);
            self.world.temp[y as usize * world::W() + x as usize] = load.temp;
            load.count -= 1;
            poured += 1;
        }
        self.shovel = if load.count == 0 { None } else { Some(load) };
        poured
    }

    pub fn shovel_dump(&mut self, at: Vec2) {
        while self.shovel.is_some() && self.shovel_pour(at, 60) > 0 {}
        if self.shovel.is_some() {
            self.say("Plus de place ici pour tout verser");
        }
    }

    /// How much a click held for `charge` seconds will let go of: a pinch for a quick click, growing the
    /// longer it is held, all of the load after about a second and a half.
    pub fn shovel_portion(load: &Load, charge: f32) -> u32 {
        let small = 8.min(load.count);
        let grown = small as f32 + (load.count - small) as f32 * (charge / 1.5).clamp(0.0, 1.0);
        (grown.round() as u32).clamp(small, load.count)
    }

    /// Mouse handling for the shovel tool. An empty shovel fills with a click (and only an empty one).
    /// A full one lets go on release: the longer the button was held, the bigger the part, up to
    /// everything it carries. As many presses as wanted, until it is empty.
    pub fn shovel_mouse(&mut self, at: Vec2, click: bool, held: bool) {
        if click {
            match self.shovel {
                None => {
                    self.checkpoint();
                    self.shovel_scoop(at);
                }
                Some(_) => {
                    self.checkpoint();
                    self.shovel_charge = Some(0.0);
                }
            }
        } else if held {
            if let Some(t) = &mut self.shovel_charge {
                *t += get_frame_time();
            }
        }
    }

    /// The button went up: the shovel lets go of the part it has been charging.
    pub fn shovel_release(&mut self, at: Vec2) {
        let Some(charge) = self.shovel_charge.take() else { return };
        let Some(load) = self.shovel else { return };
        let portion = Self::shovel_portion(&load, charge);
        if self.shovel_pour(at, portion) == 0 {
            self.say("Plus de place ici pour verser");
        }
    }

    /// The pointer of the shovel: a bubble that fills when it scoops and drains as it is poured.
    pub fn draw_shovel_cursor(&self, ui: &Ui, view: View, at: Vec2) {
        let m = ui.mouse;
        let (r, centre) = (17.0, vec2(m.x + 30.0, m.y - 26.0));
        match self.shovel {
            None => {
                // Empty: the reach of a scoop, and an empty bubble.
                let reach = (5.0 + self.brush as f32 * 0.6) * view.s;
                draw_circle_lines(m.x, m.y, reach, 1.5, Color::new(1.0, 1.0, 1.0, 0.7));
                draw_circle(centre.x, centre.y, r, Color::new(0.05, 0.06, 0.12, 0.7));
                draw_circle_lines(centre.x, centre.y, r, 2.0, Color::new(1.0, 1.0, 1.0, 0.8));
            }
            Some(load) => {
                draw_circle_lines(m.x, m.y, 8.0 * view.s, 1.5, Color::new(1.0, 1.0, 1.0, 0.7));
                draw_circle(centre.x, centre.y, r, Color::new(0.05, 0.06, 0.12, 0.8));
                // The bubble fills up (as the scoop is taken), level with how much is left.
                let frac = (load.count as f32 / load.cap.max(1) as f32).clamp(0.0, 1.0) * (self.shovel_age / 0.35).min(1.0);
                let col = science::mat_color(load.mat);
                let level = centre.y + r - 2.0 * r * frac;
                let mut y = centre.y + r;
                while y > level {
                    let dy = (y - centre.y).abs().min(r);
                    let half = (r * r - dy * dy).max(0.0).sqrt();
                    draw_line(centre.x - half, y, centre.x + half, y, 1.6, col);
                    y -= 1.4;
                }
                // While the button is held, a bright line shows how much would be let go of.
                if let Some(t) = self.shovel_charge {
                    let part = Self::shovel_portion(&load, t) as f32 / load.cap.max(1) as f32;
                    let from = (load.count as f32 / load.cap.max(1) as f32).clamp(0.0, 1.0);
                    let y = centre.y + r - 2.0 * r * (from - part).max(0.0);
                    let half = (r * r - (y - centre.y).powi(2)).max(0.0).sqrt();
                    draw_line(centre.x - half, y, centre.x + half, y, 3.0, Color::new(1.0, 1.0, 1.0, 0.95));
                    ui.text(&format!("verser {}", Self::shovel_portion(&load, t)), centre.x + r + 6.0, centre.y + 36.0, 13, Color::new(0.6, 1.0, 0.7, 1.0));
                }
                draw_circle_lines(centre.x, centre.y, r, 2.0, Color::new(1.0, 1.0, 1.0, 0.9));
                ui.text_center(&format!("{}", load.count), centre.x, centre.y + 5.0, 13, WHITE);
                ui.text(&science::info(load.mat).name.to_lowercase(), centre.x + r + 6.0, centre.y + 4.0, 13, WHITE);
                if load.temp > 120.0 {
                    ui.text(&format!("{:.0} °C", load.temp), centre.x + r + 6.0, centre.y + 20.0, 12, Color::new(1.0, 0.6, 0.3, 1.0));
                }
            }
        }
        draw_icon(Icon::Shovel, vec2(m.x + 12.0, m.y - 12.0), 26.0, WHITE);
        let _ = at;
    }
}
