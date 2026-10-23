//! The magnifying glass tool: the pointer is a lens, and a round enlarged view in the corner shows
//! the matter under its cross, with its name.

use super::*;
use crate::mechgame::gear_name;

const LENS_R: f32 = 96.0;
/// Cells across the lens (odd, so that one is at the centre).
const LENS_CELLS: i32 = 25;

impl Game {
    /// What is at `at` besides plain matter: a block, a gear, a pipe...
    fn thing_at(&self, at: Vec2) -> Option<(String, String)> {
        if let Some(i) = block_at(&self.blocks, at) {
            let b = &self.blocks[i];
            let s = b.spec();
            let rust = if b.rust > 0.02 { format!(" · rouille {:.0} %", b.rust * 100.0) } else { String::new() };
            return Some((format!("{} (bloc){rust}", s.name), s.blurb.to_string()));
        }
        if let Some(i) = self.gear_at(at) {
            let g = &self.mech.gears[i];
            let name = if g.motor.is_some() {
                "Moteur".to_string()
            } else if g.reel.is_some() {
                "Treuil".to_string()
            } else {
                gear_name(g.size).to_string()
            };
            return Some((name, format!("{:.0} tr/min · couple {:.1}", g.omega.abs() * 3600.0 / std::f32::consts::TAU, g.torque)));
        }
        if let Some(i) = self.pulley_at(at) {
            return Some(("Poulie".into(), format!("rayon {:.0}", self.mech.pulleys[i].radius)));
        }
        if let Some(i) = self.valve_at(at) {
            let v = &self.mech.plumbing.valves[i];
            return Some(("Vanne".into(), format!("ouverte à {:.0} %", v.open * 100.0)));
        }
        if let Some(p) = self.mech.plumbing.pipes.iter().find(|p| p.distance_to((at.x, at.y)).0 < 4.0) {
            let state = if p.primed { format!("plein, débit {:.2}", p.flow.abs()) } else { "vide".to_string() };
            return Some(("Tuyau".into(), state));
        }
        if let Some((i, _)) = self.rail_at_pub(at) {
            return Some(("Coulisseau".into(), format!("force {:.1}", self.mech.sliders[i].load)));
        }
        None
    }

    pub fn draw_lens(&self, ui: &Ui, view: View, at: Vec2) {
        let (cx, cy) = (at.x.floor() as i32, at.y.floor() as i32);
        let area = view.rect();
        let c = vec2(area.x + area.w - LENS_R - 26.0, area.y + area.h - LENS_R - 112.0);
        let cs = LENS_R * 2.0 / LENS_CELLS as f32;
        let half = LENS_CELLS / 2;
        let day = backdrop::sky_light(self.world.phase());
        let sky = |k: f32| (22.0 + 108.0 * k, 28.0 + 152.0 * k, 50.0 + 185.0 * k);
        let (sr, sg, sb) = sky(day);

        // A dark disc under the picture, then the enlarged cells that fall inside the round lens.
        draw_circle(c.x, c.y, LENS_R + 2.0, Color::new(0.02, 0.02, 0.05, 1.0));
        for j in -half..=half {
            for i in -half..=half {
                let (dx, dy) = (i as f32 * cs, j as f32 * cs);
                if dx * dx + dy * dy > (LENS_R - 3.0) * (LENS_R - 3.0) {
                    continue;
                }
                let (x, y) = (cx + i, cy + j);
                let col = if !(0..W as i32).contains(&x) || !(0..H as i32).contains(&y) {
                    Color::new(0.03, 0.03, 0.05, 1.0)
                } else {
                    let m = self.world.at(x, y);
                    if m == Mat::Empty {
                        Color::from_rgba(sr as u8, sg as u8, sb as u8, 255)
                    } else if self.world.raw(x, y) == m {
                        let [r, g, b, _] = self.world.color(x as usize, y as usize);
                        Color::from_rgba(r, g, b, 255)
                    } else {
                        mat_color(m)
                    }
                };
                draw_rectangle(c.x + dx - cs / 2.0, c.y + dy - cs / 2.0, cs + 0.6, cs + 0.6, col);
            }
        }
        // The rim of the lens, its handle and a glint of light.
        let handle = c + vec2(0.72, 0.72) * (LENS_R + 4.0);
        draw_line(handle.x, handle.y, handle.x + 30.0, handle.y + 30.0, 13.0, Color::from_rgba(96, 66, 44, 255));
        draw_line(handle.x, handle.y, handle.x + 30.0, handle.y + 30.0, 7.0, Color::from_rgba(150, 104, 70, 255));
        draw_circle_lines(c.x, c.y, LENS_R + 3.0, 9.0, Color::from_rgba(52, 56, 72, 255));
        draw_circle_lines(c.x, c.y, LENS_R - 1.0, 2.0, Color::from_rgba(190, 196, 214, 255));
        draw_circle(c.x - LENS_R * 0.5, c.y - LENS_R * 0.52, LENS_R * 0.2, Color::new(1.0, 1.0, 1.0, 0.1));
        // The cross on the cell being read.
        draw_rectangle_lines(c.x - cs / 2.0, c.y - cs / 2.0, cs, cs, 2.0, WHITE);
        for (a, b) in [((-1.0, 0.0), (-1.0, 0.0)), ((1.0, 0.0), (1.0, 0.0)), ((0.0, -1.0), (0.0, -1.0)), ((0.0, 1.0), (0.0, 1.0))] {
            let _ = b;
            let (from, to) = (c + vec2(a.0, a.1) * (cs * 0.8), c + vec2(a.0, a.1) * (LENS_R * 0.34));
            draw_line(from.x, from.y, to.x, to.y, 1.6, Color::new(1.0, 1.0, 1.0, 0.9));
        }

        // What it is.
        let (title, detail, fact) = match self.thing_at(at) {
            Some((t, d)) => (t, d, String::new()),
            None => {
                let m = self.world.at(cx, cy);
                let i = info(m);
                let atm = self.world.pressure_at_cell(cx, cy);
                (
                    i.name.to_string(),
                    format!("{} · {:.0} °C · {:.2} atm", i.state, self.world.temp_c(cx, cy), atm),
                    i.fact.to_string(),
                )
            }
        };
        let w = 300.0;
        let fact_h = if fact.is_empty() { 0.0 } else { ui.wrapped_height(&fact, w - 20.0, 13) + 4.0 };
        let r = Rect::new(c.x - w / 2.0 - 20.0, c.y + LENS_R + 16.0, w, 46.0 + fact_h);
        let r = Rect::new(r.x.min(area.x + area.w - w - 8.0), r.y.min(area.y + area.h - r.h - 6.0), r.w, r.h);
        rrect(r, 12.0, Color::new(0.05, 0.06, 0.12, 0.94));
        ui.text(&title, r.x + 12.0, r.y + 22.0, 19, ACCENT);
        ui.text(&detail, r.x + 12.0, r.y + 41.0, 14, TEXT);
        if !fact.is_empty() {
            ui.wrapped(&fact, r.x + 12.0, r.y + 60.0, w - 20.0, 13, DIM);
        }
    }

    /// The pointer of the loupe tool: a lens ring whose centre is the point being looked at.
    pub fn draw_lens_cursor(&self, ui: &Ui) {
        let m = ui.mouse;
        draw_line(m.x + 9.0, m.y + 9.0, m.x + 20.0, m.y + 20.0, 5.0, Color::from_rgba(150, 104, 70, 255));
        draw_circle(m.x, m.y, 12.0, Color::new(1.0, 1.0, 1.0, 0.12));
        draw_circle_lines(m.x, m.y, 12.0, 3.0, Color::from_rgba(214, 218, 230, 255));
        draw_line(m.x - 5.0, m.y, m.x + 5.0, m.y, 1.2, WHITE);
        draw_line(m.x, m.y - 5.0, m.x, m.y + 5.0, 1.2, WHITE);
    }
}
