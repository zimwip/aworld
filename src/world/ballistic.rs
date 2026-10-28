//! What flies through the air off the grid: the drops of the liquids, the clods a blast throws,
//! the debris of a car, a stick of dynamite, the water of the fire engine's jet. Each keeps what
//! it does when it lands (it joins its liquid, rolls down a slope, bounces, puts out a flame),
//! but they all fly the same way: gravity pulls them (`units::G`), the air holds them back by a
//! drag that grows as the square of their speed (`pull`), and they go through the grid a cell at
//! a time at most, so that nothing goes through a wall (`sweep`).

use super::W;
use super::H;

/// One step of flight: the body gains `g` of speed downwards (what gravity gives it in the
/// step), and the air takes from its speed in proportion to its square, g (v / v_t)², so that
/// it never falls faster than `terminal` (the speed where drag and gravity balance; infinite:
/// no drag). Any unit of speed will do, the same for the three.
pub fn pull(vx: &mut f32, vy: &mut f32, g: f32, terminal: f32) {
    *vy += g;
    let speed = (*vx * *vx + *vy * *vy).sqrt();
    let k = (1.0 - g * speed / (terminal * terminal)).max(0.0);
    *vx *= k;
    *vy *= k;
}

/// How a body's way through the grid ended (see `sweep`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Path<T> {
    /// It went the whole way.
    Clear,
    /// It left the world, past a side or the bottom (above the top it flies on).
    Out,
    /// The cell it was about to go into stopped it; it is in the last free place.
    Hit(T),
}

/// Moves a body at (`x`, `y`) (in cells) by (`dx`, `dy`), in steps no longer than a cell so
/// that it looks at every cell on its way: `stop(cell x, cell y)` says whether that cell stops
/// it (`Some`) and how. Cells above the top of the world are not looked at.
pub fn sweep<T>(x: &mut f32, y: &mut f32, dx: f32, dy: f32, mut stop: impl FnMut(i32, i32) -> Option<T>) -> Path<T> {
    let n = dx.abs().max(dy.abs()).ceil().max(1.0) as usize;
    for _ in 0..n {
        let (nx, ny) = (*x + dx / n as f32, *y + dy / n as f32);
        let (ix, iy) = (nx.floor() as i32, ny.floor() as i32);
        if ix < 0 || ix >= W() as i32 || iy >= H() as i32 {
            return Path::Out;
        }
        if iy >= 0 {
            if let Some(t) = stop(ix, iy) {
                return Path::Hit(t);
            }
        }
        (*x, *y) = (nx, ny);
    }
    Path::Clear
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::units;

    /// Dropped from rest, a body reaches its terminal speed and goes no faster; with no drag it
    /// falls as g t.
    #[test]
    fn a_falling_body_tends_to_its_terminal_speed() {
        let g = units::G * units::TICK;
        let (mut vx, mut vy) = (0.0, 0.0);
        for _ in 0..60 * 20 {
            pull(&mut vx, &mut vy, g, 9.0);
        }
        // (It reaches the terminal speed as it gains a step's gravity, and loses that to the air.)
        assert!((vy + g - 9.0).abs() < 0.01, "{vy}");
        let (mut vx, mut vy) = (0.0, 0.0);
        for _ in 0..60 {
            pull(&mut vx, &mut vy, g, f32::INFINITY);
        }
        assert!((vy - units::G).abs() < 1e-3, "{vy}");
    }

    /// A fast body does not go through a wall one cell thick: it stops in front of it.
    #[test]
    fn a_fast_body_stops_at_a_thin_wall() {
        let (mut x, mut y) = (10.5, 10.5);
        let path = sweep(&mut x, &mut y, 30.0, 0.0, |cx, _| (cx == 20).then_some(cx));
        assert_eq!(path, Path::Hit(20));
        assert!(x.floor() as i32 == 19, "{x}");
        let (mut x, mut y) = (10.5, 10.5);
        assert_eq!(sweep(&mut x, &mut y, -30.0, 0.0, |_, _| None::<()>), Path::Out);
    }
}
