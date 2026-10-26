//! Liquids under pressure. Cell by cell a liquid only falls and spreads, like grains that never
//! pile up: it cannot climb. What makes it a liquid is that it passes its pressure on. Every
//! `LEVEL_EVERY` steps the connected bodies of each liquid are found and, inside a body, liquid
//! goes from where its head (its height, plus the pressure of the gas on it) is highest to where
//! it is lowest, a few cells at a time:
//! - between two free surfaces: water climbs back up a U-tube, two vessels joined at the bottom
//!   level out, a boiler under pressure pushes its water up a pipe;
//! - out of the mouth of a full pipe below the surface: the column in the pipe is held up by the
//!   pressure behind it and does not break, and the liquid runs out of the mouth. That is a
//!   siphon once the pipe goes over a crest: it drains the tank down to the pipe's mouth, as long
//!   as the crest is not higher above the surface than the air can push the liquid up
//!   (about 10 m of water, 60 cells: see `ATM_PER_CELL`).
//!
//! Only liquid at rest carries pressure: a stream falling through the air (with air beside it)
//! is cut from the body, so a waterfall does not pump the lake into the pool under it. Each
//! liquid flows at its own pace (`fluidity`): oil a little slower than water, molten metal much
//! slower, lava slowest of all.

use super::pockets::ATM_PER_CELL;
use super::*;

/// Liquid bodies are levelled every this many steps.
pub const LEVEL_EVERY: u32 = 2;
/// A pipe or hole wider than this does not hold its column: air gets in and the liquid falls.
const HOLD_WIDTH: i32 = 8;
/// At most this many cells move in one body in one pass.
const MAX_MOVES: f32 = 24.0;

/// How readily a liquid flows, against water (1).
pub fn fluidity(m: Mat) -> f32 {
    match m {
        Mat::Oil => 0.6,
        Mat::Molten | Mat::MoltenSteel => 0.3,
        Mat::Lava => 0.12,
        _ => 1.0,
    }
}

#[inline]
fn free(m: Mat) -> bool {
    matches!(m.kind(), Kind::Empty | Kind::Gas | Kind::Flame)
}

/// A cell of a body where liquid can be taken, or a free cell where it can be put, with its head.
#[derive(Clone, Copy)]
struct Spot {
    head: f32,
    x: i32,
    y: i32,
}

impl World {
    /// Whether the liquid at (x, y), with the same liquid above it and nothing under it, is held
    /// up: it is the mouth of a narrow pipe (or hole) full of liquid, where no air can get in to
    /// take its place (a drinking straw with a finger on top), and the levelling has not let it go.
    pub(super) fn held(&self, x: i32, y: i32) -> bool {
        // Over the bottom edge of the world there is nothing to hold it against: it falls out.
        if !Self::inb(x, y) || y + 1 >= H() as i32 || self.released[Self::idx(x, y)] {
            return false;
        }
        let m = self.raw(x, y);
        self.raw(x, y - 1) == m && self.closed_run(x, y, m, HOLD_WIDTH)
    }

    /// Whether the row of liquid `m` through (x, y) is at most `width` wide and closed at both
    /// ends (by walls, or another liquid).
    fn closed_run(&self, x: i32, y: i32, m: Mat, width: i32) -> bool {
        let (mut l, mut r) = (x - 1, x + 1);
        while x - l <= width && self.raw(l, y) == m {
            l -= 1;
        }
        while r - x <= width && self.raw(r, y) == m {
            r += 1;
        }
        r - l - 1 <= width && Self::inb(l, y) && Self::inb(r, y) && !free(self.at(l, y)) && !free(self.at(r, y))
    }

    pub(super) fn level_liquids(&mut self) {
        for &i in &self.released_list {
            self.released[i] = false;
        }
        self.released_list.clear();
        let mut seen = std::mem::take(&mut self.seen);
        let mut stack = std::mem::take(&mut self.stack);
        let (mut surf, mut mouths) = (Vec::new(), Vec::new());
        seen.fill(false);
        // Seeds only in the zones that ran: a body that lies wholly in sleeping zones has not
        // changed, it is at rest.
        for zone in 0..sleep::CW() * sleep::CH() {
            if !self.zones.awake[zone] {
                continue;
            }
            let (zx, zy) = (zone % sleep::CW() * sleep::CS, zone / sleep::CW() * sleep::CS);
            for start in (zy..(zy + sleep::CS).min(H())).flat_map(|y| (zx..(zx + sleep::CS).min(W())).map(move |x| y * W() + x)) {
                let m = self.cells[start].m;
                if seen[start] || m.kind() != Kind::Liquid {
                    continue;
                }
                self.level_from(start, m, &mut seen, &mut stack, &mut surf, &mut mouths);
            }
        }
        self.seen = seen;
        self.stack = stack;
    }

    /// Finds the body of liquid `m` that cell `start` is in, and levels it.
    fn level_from(&mut self, start: usize, m: Mat, seen: &mut [bool], stack: &mut Vec<usize>, surf: &mut Vec<usize>, mouths: &mut Vec<usize>) {
        surf.clear();
        mouths.clear();
        let mut top = H() as i32;
        seen[start] = true;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = ((i % W()) as i32, (i / W()) as i32);
            top = top.min(y);
            let (above, below) = (self.at(x, y - 1), self.at(x, y + 1));
            if free(above) && below == m {
                surf.push(i);
            }
            if free(below) && above == m {
                mouths.push(i);
            }
            // Liquid with air beside it is falling or spilling: it does not pass the pressure on.
            if free(self.at(x - 1, y)) || free(self.at(x + 1, y)) {
                continue;
            }
            for (nx, ny) in Self::neighbors(x, y) {
                if Self::inb(nx, ny) {
                    let j = Self::idx(nx, ny);
                    if !seen[j] && self.cells[j].m == m {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        if !surf.is_empty() || !mouths.is_empty() {
            self.level_body(m, top, surf, mouths);
        }
    }

    /// Pressure of the gas at (x, y), or of the first gas met going on in direction `dy`: a cell
    /// that just turned to gas (the liquid levelling took it) has no pressure known yet.
    fn gas_from(&self, x: i32, y: i32, dy: i32) -> f32 {
        for k in 0..8 {
            let y = y + k * dy;
            if !free(self.at(x, y)) {
                break;
            }
            if let Some(p) = self.pocket_at(x, y) {
                return self.pockets[p].pressure;
            }
        }
        1.0
    }

    /// The first cells left and right of (x, y) along its row that are not liquid `m`.
    fn run_ends(&self, x: i32, y: i32, m: Mat) -> (i32, i32) {
        let (mut l, mut r) = (x - 1, x + 1);
        while l >= 0 && self.raw(l, y) == m {
            l -= 1;
        }
        while r < W() as i32 && self.raw(r, y) == m {
            r += 1;
        }
        (l, r)
    }

    /// Cells of `list` (sorted row by row) whose row of liquid is closed at both ends (by walls,
    /// or another liquid) and at most `width` wide, with the cell itself.
    fn closed_runs(&self, list: &[usize], m: Mat, width: i32, mut keep: impl FnMut(i32, i32)) {
        let mut run: Option<(i32, i32, i32, bool)> = None;
        for &i in list {
            let (x, y) = ((i % W()) as i32, (i / W()) as i32);
            let closed = match run {
                Some((ry, _, r, c)) if ry == y && x < r => c,
                _ => {
                    let (l, r) = self.run_ends(x, y, m);
                    let c = r - l - 1 <= width && !free(self.at(l, y)) && !free(self.at(r, y));
                    run = Some((y, l, r, c));
                    c
                }
            };
            if closed {
                keep(x, y);
            }
        }
    }

    /// Moves liquid inside one body from its highest heads to its lowest ones.
    fn level_body(&mut self, m: Mat, top: i32, surf: &mut Vec<usize>, mouths: &mut Vec<usize>) {
        let rho = density(m).max(0.3);
        // Gas pressure over the ambient one, in cells of this liquid.
        let gas = |w: &World, x: i32, y: i32, dy: i32| (w.gas_from(x, y, dy) - 1.0) / (rho * ATM_PER_CELL);
        surf.sort_unstable();
        mouths.sort_unstable();
        // Taking from a surface lowers it: only a calm one, from shore to shore (not the top of a
        // ripple, nor a heap still spreading). Putting on a surface raises it.
        let mut from = Vec::new();
        self.closed_runs(surf, m, W() as i32, |x, y| from.push(Spot { head: (H() as i32 - y) as f32 + gas(self, x, y - 1, -1), x, y }));
        let mut to: Vec<Spot> = surf
            .iter()
            .map(|&i| ((i % W()) as i32, (i / W()) as i32))
            .filter(|&(x, y)| self.at(x, y - 1) == Mat::Empty)
            .map(|(x, y)| Spot { head: (H() as i32 - y + 1) as f32 + gas(self, x, y - 1, -1), x, y: y - 1 })
            .collect();
        // The mouths of full pipes and narrow holes. The liquid in one is held up while the body
        // pushes it out (and it runs out there), or while it has nowhere to go. The highest of
        // several mouths with no surface above them all lets air in (a siphon whose end came out
        // of the tank drains). A column that would have to hang higher above the surface than the
        // pressure of the air on it can hold breaks: its mouths let go.
        let mut open = Vec::new();
        self.closed_runs(mouths, m, HOLD_WIDTH, |x, y| {
            let head = (H() as i32 - y - 1) as f32 + gas(self, x, y + 1, 1);
            open.push((x, y, head));
        });
        for &(x, y, head) in &open {
            from.push(Spot { head, x, y });
        }
        from.sort_unstable_by(|a, b| b.head.total_cmp(&a.head));
        let Some(&high) = from.first() else { return };
        let lift = self.gas_from(high.x, high.y - 1, -1) / (rho * ATM_PER_CELL);
        let broken = (H() as i32 - top) as f32 > high.head + lift;
        for &(x, y, head) in &open {
            if broken || head + 1.0 > high.head && (x, y) != (high.x, high.y) {
                let i = Self::idx(x, y);
                self.released[i] = true;
                self.released_list.push(i);
            }
        }
        if !broken {
            // A mouth well below the surface gushes: several cells at once.
            for &(x, y, head) in &open {
                let n = ((high.head - head).max(0.0).sqrt() / 3.0).clamp(1.0, 4.0) as i32;
                for k in 1..=n {
                    if !Self::inb(x, y + k) || self.at(x, y + k) != Mat::Empty {
                        break;
                    }
                    to.push(Spot { head, x, y: y + k });
                }
            }
        }
        to.sort_unstable_by(|a, b| a.head.total_cmp(&b.head));
        let Some(low) = to.first() else { return };
        // Faster the bigger the difference of head (Torricelli: speed goes as its square root).
        let budget = (high.head - low.head).max(0.0).sqrt().min(MAX_MOVES) as usize;
        let flow = fluidity(m);
        for (d, r) in from.iter().zip(to.iter()).take(budget.max(1)) {
            if d.head - r.head < 1.0 {
                break;
            }
            if flow < 1.0 && (self.rand() % 1000) as f32 >= flow * 1000.0 {
                continue;
            }
            if !Self::inb(r.x, r.y) || self.raw(d.x, d.y) != m || self.at(r.x, r.y) != Mat::Empty {
                continue;
            }
            let (i, j) = (Self::idx(d.x, d.y), Self::idx(r.x, r.y));
            let (t, fill) = (self.temp[i], self.fill[i]);
            self.set(d.x, d.y, Mat::Empty);
            self.set(r.x, r.y, m);
            self.temp[j] = t;
            // (A cell of liquid takes what it held with it.)
            self.fill[j] = fill;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    fn rect(w: &mut World, x0: i32, y0: i32, x1: i32, y1: i32, m: Mat) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                w.set(x, y, m);
            }
        }
    }

    fn count(w: &World, x0: i32, y0: i32, x1: i32, y1: i32, m: Mat) -> usize {
        (y0..=y1).flat_map(|y| (x0..=x1).map(move |x| (x, y))).filter(|&(x, y)| w.raw(x, y) == m).count()
    }

    /// Highest row holding the liquid in a column range (the surface), if any.
    fn surface(w: &World, x0: i32, x1: i32, y0: i32, y1: i32, m: Mat) -> Option<i32> {
        (y0..=y1).find(|&y| (x0..=x1).filter(|&x| w.raw(x, y) == m).count() * 2 > (x1 - x0 + 1) as usize)
    }

    pub(crate) fn dump(w: &World, x0: i32, y0: i32, x1: i32, y1: i32) -> String {
        let mut s = String::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                s.push(match w.raw(x, y) {
                    Mat::Empty => ' ',
                    Mat::Water => '~',
                    Mat::Oil => 'o',
                    Mat::Lava => 'L',
                    Mat::Stone | Mat::Brick => '#',
                    _ => '?',
                });
            }
            s.push('\n');
        }
        s
    }

    /// Two tanks 16 wide joined by a pipe along the bottom; the left one filled `rows` deep.
    fn vessels(m: Mat, rows: i32) -> World {
        let mut w = World::new_static();
        // Brick, which even fresh lava does not melt.
        rect(&mut w, 100, 300, 160, 460, Mat::Brick);
        rect(&mut w, 101, 300, 116, 450, Mat::Empty);
        rect(&mut w, 144, 300, 159, 450, Mat::Empty);
        rect(&mut w, 117, 447, 143, 450, Mat::Empty);
        rect(&mut w, 101, 451 - rows, 116, 450, m);
        w
    }

    fn levels(w: &World, m: Mat) -> (i32, i32) {
        (surface(w, 101, 116, 300, 450, m).unwrap_or(451), surface(w, 144, 159, 300, 450, m).unwrap_or(451))
    }

    #[test]
    fn water_levels_out_between_two_vessels() {
        let mut w = vessels(Mat::Water, 80);
        let mut t = 0;
        while t < 4000 {
            for _ in 0..100 {
                w.step();
            }
            t += 100;
            let (l, r) = levels(&w, Mat::Water);
            if (l - r).abs() <= 2 {
                break;
            }
        }
        let (l, r) = levels(&w, Mat::Water);
        eprintln!("water levelled in {t} steps: {l} / {r}");
        assert!((l - r).abs() <= 2, "left at {l}, right at {r}\n{}", dump(&w, 100, 360, 160, 455));
        assert!(t <= 1500, "too slow: {t} steps");
    }

    #[test]
    fn oil_levels_out_too_and_lava_much_more_slowly() {
        let mut oil = vessels(Mat::Oil, 60);
        let mut t = 0;
        while t < 4000 && { let (l, r) = levels(&oil, Mat::Oil); (l - r).abs() > 3 } {
            for _ in 0..100 {
                oil.step();
            }
            t += 100;
        }
        let (l, r) = levels(&oil, Mat::Oil);
        eprintln!("oil levelled in {t} steps: {l} / {r}");
        assert!((l - r).abs() <= 3, "oil at {l} / {r}\n{}", dump(&oil, 100, 360, 160, 455));
        // Lava (in a hot vessel, so that it does not set) has only begun to climb the other side in three times as long.
        let mut lava = vessels(Mat::Lava, 60);
        for i in 0..W() * H() {
            if matches!(lava.cells[i].m, Mat::Lava | Mat::Brick) {
                lava.temp[i] = thermal::LAVA_C;
            }
        }
        for _ in 0..3 * t {
            lava.step();
        }
        let (l, r) = levels(&lava, Mat::Lava);
        let across = count(&lava, 144, 300, 159, 450, Mat::Lava);
        eprintln!("lava after {} steps: {l} / {r}, {across} cells across", 3 * t);
        assert!(across > 0, "the lava did not get across at all\n{}", dump(&lava, 100, 360, 160, 455));
        assert!(r - l > 10, "lava as quick as oil: {l} / {r}");
    }

    /// A tank 60 deep (open to the sky through a vent), and a pipe dipping into it down to
    /// `mouth`, rising over a crest at row `crest` and coming down far below the tank to an open
    /// end over the void.
    fn siphon(primed: bool, crest: i32, mouth: i32) -> World {
        let mut w = World::new_static();
        rect(&mut w, 100, 200, 200, 420, Mat::Stone);
        rect(&mut w, 110, 240, 143, 300, Mat::Empty);
        rect(&mut w, 110, 200, 125, 239, Mat::Empty);
        rect(&mut w, 110, 250, 143, 300, Mat::Water);
        // The pipe's walls inside the tank (against its right side), its inside 2 cells wide.
        rect(&mut w, 141, crest - 2, 141, mouth, Mat::Stone);
        rect(&mut w, 144, crest - 2, 144, mouth, Mat::Stone);
        let fill = if primed { Mat::Water } else { Mat::Empty };
        rect(&mut w, 142, crest, 143, mouth, fill);
        rect(&mut w, 142, crest, 181, crest + 1, fill);
        rect(&mut w, 180, crest, 181, 420, fill);
        w
    }

    fn tank_surface(w: &World) -> i32 {
        surface(w, 110, 140, 240, 300, Mat::Water).unwrap_or(301)
    }

    #[test]
    fn a_primed_siphon_empties_the_tank_down_to_its_mouth() {
        let mut w = siphon(true, 236, 282);
        let mut t = 0;
        while t < 6000 && tank_surface(&w) < 280 {
            for _ in 0..100 {
                w.step();
            }
            t += 100;
        }
        let s = tank_surface(&w);
        eprintln!("siphon: tank down to row {s} in {t} steps\n{}", dump(&w, 100, 230, 200, 310));
        assert!(s >= 280, "the tank only fell to row {s}");
        // Then air gets in and the siphon stops: the water left under its mouth stays.
        for _ in 0..1000 {
            w.step();
        }
        let left = count(&w, 110, 240, 140, 300, Mat::Water);
        assert!(left > 300, "the tank went on draining below the mouth: {left} cells left");
    }

    #[test]
    fn a_full_pipe_ending_on_the_bottom_of_the_world_drains_out_of_it() {
        // A tank whose pipe goes down to the last row: the liquid leaves the world there.
        let mut w = World::new_static();
        let b = H() as i32 - 1;
        rect(&mut w, 300, b - 80, 340, b, Mat::Stone);
        rect(&mut w, 305, b - 80, 335, b - 20, Mat::Empty);
        rect(&mut w, 319, b - 19, 320, b, Mat::Empty);
        for m in [Mat::Water, Mat::Oil] {
            rect(&mut w, 305, b - 60, 335, b - 20, m);
            rect(&mut w, 319, b - 19, 320, b, m);
            let before = count(&w, 305, b - 80, 335, b - 20, m);
            for _ in 0..1500 {
                w.step();
            }
            let after = count(&w, 305, b - 80, 335, b - 20, m);
            assert!(after * 4 < before * 3, "{m:?}: {before} -> {after}");
        }
    }

    #[test]
    fn an_empty_siphon_does_not_drain_the_tank() {
        let mut w = siphon(false, 236, 282);
        let before = count(&w, 110, 240, 150, 300, Mat::Water);
        for _ in 0..2000 {
            w.step();
        }
        let after = count(&w, 110, 240, 150, 300, Mat::Water);
        eprintln!("tank {before} -> {after}\n{}", dump(&w, 100, 230, 200, 310));
        assert!(after * 100 >= before * 97, "lost {before} -> {after}");
    }

    #[test]
    fn the_air_cannot_push_water_over_a_crest_more_than_60_cells_up() {
        // The crest is 25 cells over the water and the mouth 69 under it: the siphon runs until the
        // surface is 60 cells (about 10 m) under the crest, then the column breaks (and what was
        // in the pipe on the tank's side falls back into it).
        let mut w = siphon(true, 225, 294);
        let mut lowest = 0;
        for _ in 0..50 {
            for _ in 0..100 {
                w.step();
            }
            lowest = lowest.max(tank_surface(&w));
        }
        eprintln!("high siphon: tank down to row {lowest} at most");
        assert!((280..=290).contains(&lowest), "stopped at row {lowest}");
    }
}


