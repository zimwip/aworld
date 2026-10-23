//! Sky, sun, moon, stars and the landscape behind the world (one per biome),
//! all following the time of day and the season.

use crate::render::View;
use crate::world::{Biome, Season};
use macroquad::prelude::*;

fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::new(a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t, 1.0)
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba(r, g, b, 255)
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Daylight 0..1 for a time of day (0 sunrise, 0.25 noon, 0.5 sunset).
pub fn daylight(phase: f32) -> f32 {
    smooth(0.0, 0.3, (phase * std::f32::consts::TAU).sin())
}

/// How strongly a low sun colours the sky and the clouds (1 when it touches the horizon, fading
/// over the hour or so either side, before sunrise and after sunset too), and where across the
/// world (0 to 1) that light comes from: the sun itself, or the point where it rose or set.
pub fn twilight(phase: f32) -> (f32, f32) {
    let s = (phase * std::f32::consts::TAU).sin();
    let g = smooth(0.0, 1.0, 1.0 - s.abs() / 0.28);
    let x = if phase < 0.5 { 0.08 + 0.84 * phase / 0.5 } else if phase < 0.75 { 0.92 } else { 0.08 };
    (g, x)
}

/// How bright the sky looks: the daylight, but around sunrise and sunset the glow of the low sun
/// keeps it from going dark all at once.
pub fn sky_light(phase: f32) -> f32 {
    daylight(phase).max(0.55 * twilight(phase).0)
}

/// Dims a colour toward a cold blue at night.
fn lit(c: Color, day: f32) -> Color {
    mix(rgb(14, 20, 44), c, 0.28 + 0.72 * day)
}

fn season_ground(season: Season) -> (Color, Color) {
    match season {
        Season::Spring => (rgb(132, 196, 110), rgb(104, 170, 92)),
        Season::Summer => (rgb(104, 170, 84), rgb(78, 142, 72)),
        Season::Autumn => (rgb(196, 150, 70), rgb(160, 104, 58)),
        Season::Winter => (rgb(226, 232, 240), rgb(190, 202, 218)),
    }
}

pub fn draw_sky(v: View, biome: Biome, phase: f32, wind: f32, time: f32, season: Season) {
    let r = v.rect();
    let tau = std::f32::consts::TAU;
    let s = (phase * tau).sin();
    let day = sky_light(phase);
    let glow = (1.0 - s.abs() / 0.28).clamp(0.0, 1.0);

    // Sky gradient. At dawn and dusk the light crosses so much air that only its reds get through
    // near the horizon: orange low down, pink above it, the zenith turning violet.
    let (dusk, sun_x) = twilight(phase);
    let top = mix(mix(rgb(6, 10, 34), rgb(70, 120, 214), day), rgb(62, 54, 128), dusk * 0.45);
    let mid = mix(mix(rgb(14, 22, 60), rgb(150, 200, 238), day), rgb(226, 140, 150), dusk * 0.6);
    let low = mix(mix(rgb(30, 34, 74), rgb(255, 224, 182), day), rgb(255, 130, 70), glow * 0.85);
    let bands = 48;
    for i in 0..bands {
        let t = i as f32 / bands as f32;
        let c = if t < 0.6 { mix(top, mid, t / 0.6) } else { mix(mid, low, (t - 0.6) / 0.4) };
        draw_rectangle(r.x, r.y + r.h * t, r.w, r.h / bands as f32 + 1.0, c);
    }

    // The glow around the low sun: a wide golden-to-rose halo lying along the horizon, on the side
    // where the sun rises or sets (and lingering there a while after it has set).
    if dusk > 0.01 {
        let (hx, hy) = (r.x + r.w * sun_x, r.y + r.h * 0.56);
        for i in 0..48 {
            let t = i as f32 / 47.0;
            let c = mix(rgb(236, 110, 130), rgb(255, 214, 120), t);
            let rad = r.h * (0.95 - 0.85 * t);
            draw_ellipse(hx, hy, rad * 1.9, rad, 0.0, Color::new(c.r, c.g, c.b, 0.027 * dusk));
        }
    }

    // Stars fade in with the night. They twinkle because the air between them and us is turbulent:
    // the lower the star (the more air its light crosses) and the stronger the wind, the more it
    // flickers, flashing colours near the horizon. The planets (the first few) shine steadily:
    // they are discs, not points, so the turbulence averages out.
    let night = 1.0 - day;
    if night > 0.05 {
        for k in 0..110 {
            let h = (k as f32 * 12.9898).sin() * 43758.547;
            let (fx, fy) = (h.fract().abs(), ((h * 1.7).fract()).abs());
            let (x, y) = (r.x + fx * r.w, r.y + fy * r.h * 0.6);
            if k < 3 {
                let planet = [(1.0, 0.95, 0.8), (1.0, 0.75, 0.6), (0.95, 0.97, 1.0)][k];
                draw_circle(x, y, 2.0, Color::new(planet.0, planet.1, planet.2, night * 0.95));
                continue;
            }
            let k = k as f32;
            let turb = (0.25 + 0.75 * fy) * (0.6 + 0.8 * wind.abs());
            let flick = (time * (9.0 + k % 7.0) + k).sin() * (time * (5.3 + k % 5.0) + 2.0 * k).sin();
            let tw = (1.0 - turb * (0.5 + 0.5 * flick)).max(0.05);
            // Each colour bends a little differently in the moving air.
            let hue = (time * 7.0 + k * 1.3).sin() * turb * 0.5;
            let col = Color::new(1.0 - hue.max(0.0) * 0.6, 1.0 - hue.abs() * 0.3, 0.95 + hue.min(0.0) * 0.6, night * 0.85 * tw);
            draw_circle(x, y, 0.8 + (k as i32 % 3) as f32 * 0.5 + 0.4 * (1.0 - tw), col);
        }
    }

    // Sun by day, moon by night, each following an arc.
    let (u, is_sun) = if phase < 0.5 { (phase / 0.5, true) } else { ((phase - 0.5) / 0.5, false) };
    let cx = r.x + r.w * (0.08 + 0.84 * u);
    let cy = r.y + r.h * (0.64 - 0.52 * (std::f32::consts::PI * u).sin());
    if is_sun {
        let warm = mix(rgb(255, 250, 220), rgb(255, 150, 70), glow);
        draw_circle(cx, cy, r.h * 0.085, Color::new(warm.r, warm.g, warm.b, 0.25));
        draw_circle(cx, cy, r.h * 0.05, warm);
    } else {
        draw_circle(cx, cy, r.h * 0.07, Color::new(0.8, 0.85, 1.0, 0.12));
        draw_circle(cx, cy, r.h * 0.04, rgb(238, 240, 250));
        draw_circle(cx - r.h * 0.012, cy - r.h * 0.006, r.h * 0.008, rgb(214, 218, 232));
        draw_circle(cx + r.h * 0.014, cy + r.h * 0.012, r.h * 0.006, rgb(220, 224, 236));
    }

    // Wind streaks drifting across the sky.
    for k in 0..7 {
        let speed = wind * 40.0 + wind.signum() * 6.0;
        let x = ((k as f32 * 0.37 + time * speed * 0.004) % 1.2 + 1.2) % 1.2 - 0.1;
        let y = r.y + r.h * (0.1 + 0.07 * k as f32);
        let len = 30.0 + 40.0 * wind.abs();
        draw_line(r.x + x * r.w, y, r.x + x * r.w + len * wind.signum(), y, 1.2, Color::new(1.0, 1.0, 1.0, 0.10 + 0.12 * wind.abs()));
    }

    match biome {
        Biome::Mountain => draw_mountains(v, day, season),
        Biome::Countryside => draw_countryside(v, day, season, time, wind),
        Biome::Sea => draw_sea(v, day, phase, time),
    }
}

fn draw_mountains(v: View, day: f32, season: Season) {
    let r = v.rect();
    let step = 3.0;
    let horizon = r.y + r.h * 0.72;
    let tri = |t: f32| 1.0 - (2.0 * (t - t.floor()) - 1.0).abs();
    let height = |u: f32| 0.5 * (0.62 * tri(u * 3.0 + 0.15) + 0.38 * tri(u * 7.3 + 0.6));
    let snow_alt = horizon - r.h * if season == Season::Winter { 0.12 } else { 0.24 };
    let mut x = 0.0;
    while x < r.w {
        let u = x / r.w;
        let top = horizon - height(u) * r.h * 0.66;
        let slope = (height(u + 0.003) - height(u - 0.003)) * 60.0;
        let light = (0.5 - slope * 0.5).clamp(0.0, 1.0);
        let rock = lit(mix(rgb(96, 110, 150), rgb(166, 180, 212), light), day);
        draw_rectangle(r.x + x, top, step + 1.0, horizon - top + r.h * 0.1, rock);
        if top < snow_alt {
            draw_rectangle(r.x + x, top, step + 1.0, (snow_alt - top) * 1.1, lit(rgb(245, 249, 255), day));
        }
        x += step;
    }
    draw_hill_layer(v, 0.78, 0.04, 6.0, lit(rgb(70, 110, 96), day).into(), season, day, 0.0);
}

fn draw_hill_layer(v: View, base: f32, amp: f32, freq: f32, tint: Color, season: Season, day: f32, phase_shift: f32) {
    let r = v.rect();
    let (far, near) = season_ground(season);
    let col = lit(mix(mix(far, near, 0.5), tint, 0.25), day);
    let step = 6.0;
    let mut x = 0.0;
    while x < r.w {
        let u = x / r.w;
        let h = (base + amp * (u * freq + phase_shift).sin() + amp * 0.5 * (u * freq * 2.3 + 1.0 + phase_shift).sin()) * r.h;
        draw_rectangle(r.x + x, r.y + h, step + 1.0, r.h - h, col);
        x += step;
    }
}

fn draw_countryside(v: View, day: f32, season: Season, time: f32, wind: f32) {
    let r = v.rect();
    let (far, near) = season_ground(season);
    // Two rolling hill layers; the far one carries striped fields.
    draw_hill_layer(v, 0.66, 0.05, 5.0, lit(far, day), season, day, 0.0);
    let step = 6.0;
    let mut x = 0.0;
    while x < r.w {
        let u = x / r.w;
        let h = (0.66 + 0.05 * (u * 5.0).sin() + 0.025 * (u * 11.5 + 1.0).sin()) * r.h;
        let stripe = ((x / 22.0) as i32 + (u * 9.0) as i32) % 2 == 0;
        let field = if stripe { mix(far, rgb(224, 196, 96), 0.35) } else { mix(far, near, 0.4) };
        draw_rectangle(r.x + x, r.y + h, step + 1.0, 14.0, lit(field, day));
        x += step;
    }
    draw_hill_layer(v, 0.76, 0.04, 8.0, lit(near, day), season, day, 1.7);

    // A windmill on the far hill; its blades turn with the wind.
    let mx = r.x + r.w * 0.2;
    let my = r.y + r.h * (0.66 + 0.05 * (0.2f32 * 5.0).sin() + 0.025 * (0.2f32 * 11.5 + 1.0).sin());
    let body = lit(rgb(236, 226, 206), day);
    draw_triangle(vec2(mx - 10.0, my + 2.0), vec2(mx + 10.0, my + 2.0), vec2(mx + 6.0, my - 40.0), body);
    draw_triangle(vec2(mx - 10.0, my + 2.0), vec2(mx - 6.0, my - 40.0), vec2(mx + 6.0, my - 40.0), body);
    draw_triangle(vec2(mx - 8.0, my - 40.0), vec2(mx + 8.0, my - 40.0), vec2(mx, my - 52.0), lit(rgb(176, 84, 60), day));
    let spin = time * (0.4 + wind.abs() * 2.2);
    for k in 0..4 {
        let a = spin + k as f32 * std::f32::consts::FRAC_PI_2;
        let tip = vec2(mx + a.cos() * 30.0, my - 40.0 + a.sin() * 30.0);
        draw_line(mx, my - 40.0, tip.x, tip.y, 3.0, lit(rgb(250, 248, 240), day));
    }
    // A farm.
    let fx = r.x + r.w * 0.72;
    let fy = r.y + r.h * (0.66 + 0.05 * (0.72f32 * 5.0).sin() + 0.025 * (0.72f32 * 11.5 + 1.0).sin());
    draw_rectangle(fx, fy - 14.0, 26.0, 16.0, lit(rgb(196, 70, 60), day));
    draw_triangle(vec2(fx - 3.0, fy - 14.0), vec2(fx + 29.0, fy - 14.0), vec2(fx + 13.0, fy - 26.0), lit(rgb(110, 60, 50), day));
    draw_rectangle(fx + 10.0, fy - 8.0, 6.0, 10.0, lit(rgb(236, 226, 206), day));
    if day < 0.5 {
        draw_rectangle(fx + 4.0, fy - 10.0, 3.0, 3.0, Color::new(1.0, 0.9, 0.4, 1.0 - day * 2.0));
    }
}

fn draw_sea(v: View, day: f32, phase: f32, time: f32) {
    let r = v.rect();
    let horizon = r.y + r.h * 0.66;
    // Open water to the horizon, deeper blue toward the viewer.
    let top = lit(rgb(96, 170, 214), day);
    let bottom = lit(rgb(24, 84, 150), day);
    let bands = 18;
    for i in 0..bands {
        let t = i as f32 / bands as f32;
        draw_rectangle(r.x, horizon + (r.h - (horizon - r.y)) * t, r.w, (r.h - (horizon - r.y)) / bands as f32 + 1.0, mix(top, bottom, t));
    }
    // Faraway islands.
    for (cx, w, h) in [(0.18, 0.12, 0.05), (0.55, 0.09, 0.035), (0.86, 0.14, 0.06)] {
        let (x0, x1) = (r.x + r.w * (cx - w), r.x + r.w * (cx + w));
        let top = horizon - r.h * h;
        draw_triangle(vec2(x0, horizon), vec2(x1, horizon), vec2((x0 + x1) / 2.0, top), lit(rgb(72, 128, 98), day));
    }
    // Lighthouse.
    let lx = r.x + r.w * 0.40;
    draw_rectangle(lx - 4.0, horizon - 36.0, 8.0, 36.0, lit(rgb(240, 240, 240), day));
    draw_rectangle(lx - 4.0, horizon - 26.0, 8.0, 6.0, lit(rgb(200, 60, 56), day));
    draw_rectangle(lx - 5.0, horizon - 42.0, 10.0, 6.0, lit(rgb(40, 40, 50), day));
    if day < 0.6 {
        let a = (time * 1.4).sin() * 0.5 + 0.5;
        draw_circle(lx, horizon - 39.0, 4.0, Color::new(1.0, 0.95, 0.5, (1.0 - day) * (0.5 + a * 0.5)));
        draw_triangle(vec2(lx, horizon - 39.0), vec2(lx + 160.0, horizon - 70.0), vec2(lx + 160.0, horizon - 10.0), Color::new(1.0, 0.95, 0.6, 0.12 * (1.0 - day) * a));
    }
    // A sailing boat drifting along.
    let bx = r.x + (((time * 6.0) % (r.w + 80.0)) - 40.0);
    let by = horizon + 14.0;
    draw_triangle(vec2(bx - 10.0, by), vec2(bx + 12.0, by), vec2(bx + 8.0, by + 6.0), lit(rgb(120, 80, 56), day));
    draw_triangle(vec2(bx, by - 2.0), vec2(bx, by - 26.0), vec2(bx + 14.0, by - 2.0), lit(rgb(250, 250, 250), day));
    // Glitter on the water under the sun or moon.
    let u = if phase < 0.5 { phase / 0.5 } else { (phase - 0.5) / 0.5 };
    let gx = r.x + r.w * (0.08 + 0.84 * u);
    for k in 0..26 {
        let y = horizon + 4.0 + k as f32 * 7.5;
        let spread = 6.0 + k as f32 * 2.2;
        let off = ((time * 3.0 + k as f32 * 1.3).sin()) * spread * 0.6;
        draw_line(gx + off - spread * 0.5, y, gx + off + spread * 0.5, y, 1.5, Color::new(1.0, 1.0, 0.9, 0.35 * (day.max(0.25))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twilight_glows_at_sunrise_and_sunset_on_the_sun_side_and_not_at_noon_or_midnight() {
        let (dawn, x_dawn) = twilight(0.0);
        let (dusk, x_dusk) = twilight(0.5);
        assert!(dawn > 0.99 && dusk > 0.99);
        assert!(x_dawn < 0.2 && x_dusk > 0.8);
        // Just before sunrise and just after sunset the glow is already, or still, there.
        assert!(twilight(0.99).0 > 0.5 && twilight(0.99).1 < 0.2);
        assert!(twilight(0.51).0 > 0.5 && twilight(0.51).1 > 0.8);
        assert_eq!(twilight(0.25).0, 0.0);
        assert_eq!(twilight(0.75).0, 0.0);
        // The sky does not go dark the moment the sun touches the horizon.
        assert!(daylight(0.5) < 0.05 && sky_light(0.5) > 0.5);
        assert_eq!(sky_light(0.25), daylight(0.25));
    }
}
