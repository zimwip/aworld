//! The selection tool: pick an element, move it, drag its ends to resize it, delete it.

use super::*;
use crate::mechgame::snap_to_axes;

/// What is selected.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sel {
    Block(u32),
    Member(u32),
    Gear(u32),
    Pulley(u32),
    Slider(u32),
    Pipe(u32),
    Valve(u32),
    Pump(u32),
    Cable(usize),
    /// A car on the road (by id): selecting it shows its settings.
    Car(u32),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Grab {
    Body,
    /// One end of a line-like element (0 or 1).
    End(u8),
}

/// A drag in progress: what, how it was grabbed, where the pointer started and what the geometry was.
#[derive(Clone, Copy, Debug)]
pub struct Drag {
    pub sel: Sel,
    pub grab: Grab,
    pub start: Vec2,
    pub orig: [Vec2; 2],
    pub moved: bool,
}

const HANDLE: f32 = 7.0;

impl Game {
    /// The element under `at` (small things first).
    pub fn select_pick(&self, at: Vec2) -> Option<Sel> {
        if let Some(c) = self.cars.iter().find(|c| (c.p - at).length() < c.kind.model().hull_half * car::S + 10.0) {
            return Some(Sel::Car(c.id));
        }
        if let Some(i) = self.gear_at(at) {
            return Some(Sel::Gear(self.mech.gears[i].uid));
        }
        if let Some(i) = self.pulley_at(at) {
            return Some(Sel::Pulley(self.mech.pulleys[i].uid));
        }
        if let Some(i) = self.pump_at(at) {
            return Some(Sel::Pump(self.mech.plumbing.pumps[i].uid));
        }
        if let Some(i) = self.valve_at(at) {
            return Some(Sel::Valve(self.mech.plumbing.valves[i].uid));
        }
        if let Some(i) = block_at(&self.blocks, at) {
            return Some(Sel::Block(self.blocks[i].uid));
        }
        if let Some(m) = self.members.iter().find(|m| m.distance_to(at) < m.mat.thickness() / 2.0 + 3.0) {
            return Some(Sel::Member(m.uid));
        }
        if let Some((i, _)) = self.rail_at_pub(at) {
            return Some(Sel::Slider(self.mech.sliders[i].uid));
        }
        if let Some(p) = self.mech.plumbing.pipes.iter().find(|p| p.distance_to((at.x, at.y)).0 < 4.0) {
            return Some(Sel::Pipe(p.uid));
        }
        if let Some(i) = self.cables.iter().position(|c| c.distance_to(at) < 4.0) {
            return Some(Sel::Cable(i));
        }
        None
    }

    /// The two ends of a line-like element.
    pub fn sel_ends(&self, sel: Sel) -> Option<(Vec2, Vec2)> {
        match sel {
            Sel::Member(u) => self.members.iter().find(|m| m.uid == u).map(|m| (m.p0, m.p1)),
            Sel::Slider(u) => self.mech.sliders.iter().find(|s| s.uid == u).map(|s| (s.origin, s.origin + s.dir * s.len)),
            Sel::Pipe(u) => self.mech.plumbing.pipes.iter().find(|p| p.uid == u).map(|p| (Vec2::new(p.a.0, p.a.1), Vec2::new(p.b.0, p.b.1))),
            Sel::Cable(i) => {
                let c = self.cables.get(i)?;
                Some((anchor_world(&c.a, &self.blocks, &self.members)?, anchor_world(&c.b, &self.blocks, &self.members)?))
            }
            _ => None,
        }
    }

    /// The centre of a point-like element.
    pub fn sel_centre(&self, sel: Sel) -> Option<Vec2> {
        match sel {
            Sel::Block(u) => self.blocks.iter().find(|b| b.uid == u).map(|b| b.pos),
            Sel::Gear(u) => self.mech.gears.iter().find(|g| g.uid == u).map(|g| g.pos),
            Sel::Pulley(u) => self.mech.pulleys.iter().find(|p| p.uid == u).map(|p| p.pos),
            Sel::Valve(u) => self.mech.plumbing.valves.iter().find(|v| v.uid == u).and_then(|v| self.valve_world(v)),
            Sel::Pump(u) => self.mech.plumbing.pumps.iter().find(|p| p.uid == u).and_then(|p| self.pump_world(p)),
            Sel::Car(id) => self.cars.iter().find(|c| c.id == id).map(|c| c.p),
            _ => None,
        }
    }

    /// Which end handle of the selection (if any) is under `at`.
    fn sel_handle(&self, at: Vec2) -> Option<u8> {
        let (a, b) = self.sel_ends(self.selection?)?;
        if (a - at).length() <= HANDLE {
            Some(0)
        } else if (b - at).length() <= HANDLE {
            Some(1)
        } else {
            None
        }
    }

    /// Mouse handling for the selection tool: press picks (or grabs a handle), dragging moves or resizes.
    pub fn select_mouse(&mut self, at: Vec2, click: bool, left: bool, shift: bool) {
        if click {
            if self.running {
                self.say("Mets en pause et reviens en arrière pour modifier la construction");
                self.selection = self.select_pick(at);
                return;
            }
            if let (Some(sel), Some(end)) = (self.selection, self.sel_handle(at)) {
                let (a, b) = self.sel_ends(sel).unwrap();
                self.drag = Some(Drag { sel, grab: Grab::End(end), start: at, orig: [a, b], moved: false });
                return;
            }
            self.selection = self.select_pick(at);
            if let Some(Sel::Car(id)) = self.selection {
                if let Some(c) = self.cars.iter().find(|c| c.id == id) {
                    self.model = c.kind;
                    self.selected_car = Some(id);
                    self.car_remote = true;
                    self.selected_motor = None;
                }
            }
            self.drag = self.selection.map(|sel| {
                let orig = match (self.sel_ends(sel), self.sel_centre(sel)) {
                    (Some((a, b)), _) => [a, b],
                    (None, Some(c)) => [c, c],
                    _ => [at, at],
                };
                // Grabbing a line-like element by one of its ends resizes it.
                let grab = if self.sel_ends(sel).is_some() {
                    match self.sel_handle(at) {
                        Some(e) => Grab::End(e),
                        None => Grab::Body,
                    }
                } else {
                    Grab::Body
                };
                Drag { sel, grab, start: at, orig, moved: false }
            });
            return;
        }
        if left && !self.running {
            if let Some(d) = self.drag {
                self.select_drag(d, at, shift);
            }
        }
    }

    /// Applies a drag to the element: moves it, or moves one end of it.
    pub fn select_drag(&mut self, mut d: Drag, at: Vec2, shift: bool) {
        let delta = (at - d.start).round();
        if !d.moved && delta.length() < 2.0 {
            return;
        }
        if !d.moved {
            self.checkpoint();
            d.moved = true;
            self.drag = Some(d);
        }
        let changed = match (d.grab, d.sel) {
            (Grab::Body, Sel::Block(u)) => self.move_block(u, d.orig[0] + delta, shift),
            (Grab::Body, Sel::Gear(u)) => self.move_gear(u, d.orig[0] + delta),
            (Grab::Body, Sel::Pulley(u)) => self.move_pulley(u, d.orig[0] + delta),
            (Grab::Body, Sel::Member(u)) => self.set_member_ends(u, d.orig[0] + delta, d.orig[1] + delta, [true, true]),
            (Grab::End(e), Sel::Member(u)) => {
                let (mut p0, mut p1) = (d.orig[0], d.orig[1]);
                let fixed = if e == 0 { p1 } else { p0 };
                let moved = fixed + (at - fixed).clamp_length_max(MEMBER_MAX_LEN);
                if e == 0 {
                    p0 = moved;
                } else {
                    p1 = moved;
                }
                self.set_member_ends(u, p0, p1, [e == 0, e == 1])
            }
            (Grab::Body, Sel::Slider(u)) => self.set_rail_ends(u, d.orig[0] + delta, d.orig[1] + delta, [true, true], shift),
            (Grab::End(e), Sel::Slider(u)) => {
                let fixed = if e == 0 { d.orig[1] } else { d.orig[0] };
                let (anchor, p) = self.rail_end(fixed, at);
                let _ = anchor;
                let (p0, p1) = if e == 0 { (p, d.orig[1]) } else { (d.orig[0], p) };
                self.set_rail_ends(u, p0, p1, [e == 0, e == 1], shift)
            }
            (Grab::Body, Sel::Pipe(u)) => self.set_pipe_ends(u, d.orig[0] + delta, d.orig[1] + delta),
            (Grab::End(e), Sel::Pipe(u)) => {
                let fixed = if e == 0 { d.orig[1] } else { d.orig[0] };
                let p = self.pipe_end(fixed, at);
                let (p0, p1) = if e == 0 { (p, d.orig[1]) } else { (d.orig[0], p) };
                self.set_pipe_ends(u, p0, p1)
            }
            (Grab::Body, Sel::Valve(u)) | (Grab::Body, Sel::Pump(u)) => self.slide_on_pipe(d.sel, u, at),
            (Grab::End(e), Sel::Cable(i)) => self.reattach_cable(i, e, at),
            _ => false,
        };
        if changed {
            self.refresh_member_ends();
            self.refit_rods();
            self.refresh_mask();
        }
    }

    fn move_block(&mut self, uid: u32, to: Vec2, shift: bool) -> bool {
        let Some(i) = self.blocks.iter().position(|b| b.uid == uid) else { return false };
        let others: Vec<Block> = self.blocks.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, b)| b.clone()).collect();
        let (kind, angle) = (self.blocks[i].kind, self.blocks[i].angle);
        let pos = if shift { to } else { clip(&others, kind, angle, snap(kind, 0, to)) };
        let mut cand = self.blocks[i].clone();
        cand.pos = pos;
        if pos == self.blocks[i].pos || !placement_ok(&self.world, &self.blocks, &cand, Some(i)) {
            return false;
        }
        self.blocks[i].pos = pos;
        true
    }

    fn move_gear(&mut self, uid: u32, to: Vec2) -> bool {
        let Some(i) = self.mech.gear_index(uid) else { return false };
        let size = self.mech.gears[i].size;
        // Clip it onto the teeth of a neighbour, but not onto itself.
        let saved = self.mech.gears.remove(i);
        let c = self.gear_spot(size, to);
        let free = self.mech.gears.iter().all(|g| (g.pos - c).length() >= g.size.radius() + size.radius() - 1.5);
        let mount = self.support_for_pub(c);
        let mut g = saved;
        if free {
            g.pos = c;
            g.mount = mount;
        }
        self.mech.gears.insert(i, g);
        free
    }

    fn move_pulley(&mut self, uid: u32, to: Vec2) -> bool {
        let Some(i) = self.mech.pulleys.iter().position(|p| p.uid == uid) else { return false };
        let r = self.mech.pulleys[i].radius;
        if self.mech.pulleys.iter().enumerate().any(|(j, p)| j != i && (p.pos - to).length() < p.radius + r) {
            return false;
        }
        let mount = self.support_for_pub(to);
        self.mech.pulleys[i].pos = to;
        self.mech.pulleys[i].mount = mount;
        true
    }

    /// Sets the end points of a rod (tige), re-pinning the ends that moved onto what is there now.
    fn set_member_ends(&mut self, uid: u32, p0: Vec2, p1: Vec2, moved: [bool; 2]) -> bool {
        let Some(i) = self.members.iter().position(|m| m.uid == uid) else { return false };
        if (p1 - p0).length() < 12.0 {
            return false;
        }
        let pin = |this: &Game, p: Vec2| -> Anchor {
            // Pinned to another rod's end or a block or the ground if one is right there, else loose.
            let others: Vec<Member> = this.members.iter().filter(|m| m.uid != uid).cloned().collect();
            node_at(&this.world, &this.blocks, &others, p).filter(|a| anchor_world(a, &this.blocks, &others).map_or(false, |q| (q - p).length() < 9.0)).unwrap_or(Anchor::Free(p))
        };
        let (a, b) = (if moved[0] { pin(self, p0) } else { self.members[i].a }, if moved[1] { pin(self, p1) } else { self.members[i].b });
        let m = &mut self.members[i];
        m.p0 = p0;
        m.p1 = p1;
        m.a = a;
        m.b = b;
        true
    }

    fn set_rail_ends(&mut self, uid: u32, p0: Vec2, p1: Vec2, moved: [bool; 2], _shift: bool) -> bool {
        let Some(i) = self.mech.sliders.iter().position(|s| s.uid == uid) else { return false };
        if (p1 - p0).length() < 24.0 {
            return false;
        }
        let (a0, a1) = (
            if moved[0] { self.support_for_pub(p0) } else { self.mech.sliders[i].ends.0 },
            if moved[1] { self.support_for_pub(p1) } else { self.mech.sliders[i].ends.1 },
        );
        let sl = &mut self.mech.sliders[i];
        sl.ends = (a0, a1);
        sl.set_ends(p0, p1);
        true
    }

    fn set_pipe_ends(&mut self, uid: u32, p0: Vec2, p1: Vec2) -> bool {
        if (p1 - p0).length() < 8.0 {
            return false;
        }
        let Some(p) = self.mech.plumbing.pipes.iter_mut().find(|p| p.uid == uid) else { return false };
        p.a = (p0.x, p0.y);
        p.b = (p1.x, p1.y);
        true
    }

    /// A valve or a pump slides along its pipe.
    fn slide_on_pipe(&mut self, sel: Sel, uid: u32, at: Vec2) -> bool {
        let pipe_uid = match sel {
            Sel::Valve(_) => self.mech.plumbing.valves.iter().find(|v| v.uid == uid).map(|v| v.pipe),
            _ => self.mech.plumbing.pumps.iter().find(|p| p.uid == uid).map(|p| p.pipe),
        };
        let Some(pipe) = pipe_uid.and_then(|u| self.mech.plumbing.pipes.iter().find(|p| p.uid == u)) else { return false };
        let t = pipe.distance_to((at.x, at.y)).1.clamp(0.06, 0.94);
        match sel {
            Sel::Valve(_) => {
                if let Some(v) = self.mech.plumbing.valves.iter_mut().find(|v| v.uid == uid) {
                    v.t = t;
                }
            }
            _ => {
                if let Some(p) = self.mech.plumbing.pumps.iter_mut().find(|p| p.uid == uid) {
                    p.t = t;
                }
            }
        }
        true
    }

    /// Moves one end of a cable onto whatever is under the pointer (or leaves it loose in the air); the
    /// rope is cut to the new length.
    fn reattach_cable(&mut self, i: usize, end: u8, at: Vec2) -> bool {
        let Some(c) = self.cables.get(i) else { return false };
        let other = if end == 0 { c.b } else { c.a };
        let anchor = node_at(&self.world, &self.blocks, &self.members, at).unwrap_or(Anchor::Free(at));
        let (Some(pa), Some(pb)) = (anchor_world(&anchor, &self.blocks, &self.members), anchor_world(&other, &self.blocks, &self.members)) else { return false };
        let len = (pa - pb).length() * 1.06;
        if len < 10.0 || matches!((&anchor, &other), (Anchor::Free(_), Anchor::Free(_))) {
            return false;
        }
        let c = &mut self.cables[i];
        if end == 0 {
            c.a = anchor;
        } else {
            c.b = anchor;
        }
        c.len = len;
        true
    }

    /// Rods pinned to a block, a rod end or a gear follow what they are pinned to.
    pub fn refresh_member_ends(&mut self) {
        for _ in 0..2 {
            for i in 0..self.members.len() {
                let (a, b) = (self.members[i].a, self.members[i].b);
                if !matches!(a, Anchor::Free(_) | Anchor::Ground(_)) {
                    if let Some(p) = anchor_world(&a, &self.blocks, &self.members) {
                        self.members[i].p0 = p;
                    }
                }
                if !matches!(b, Anchor::Free(_) | Anchor::Ground(_)) {
                    if let Some(p) = anchor_world(&b, &self.blocks, &self.members) {
                        self.members[i].p1 = p;
                    }
                }
            }
        }
    }

    /// Connecting rods keep the length they are given by their two ends, so that a moved gear or rail
    /// leaves them joined rather than stretched.
    pub fn refit_rods(&mut self) {
        for r in 0..self.mech.rods.len() {
            let (ea, eb) = (self.mech.rods[r].a, self.mech.rods[r].b);
            if let (Some(a), Some(b)) = (self.rod_end_world(&ea), self.rod_end_world(&eb)) {
                let len = (a - b).length();
                if len >= 6.0 {
                    self.mech.rods[r].len = len;
                }
            }
        }
    }

    /// Removes what is selected.
    pub fn select_delete(&mut self) {
        let Some(sel) = self.selection.take() else { return };
        self.drag = None;
        if self.running {
            self.say("Mets en pause et reviens en arrière pour modifier la construction");
            self.selection = Some(sel);
            return;
        }
        self.checkpoint();
        match sel {
            Sel::Block(u) => {
                if let Some(i) = self.blocks.iter().position(|b| b.uid == u) {
                    self.remove_block(i);
                }
            }
            Sel::Member(u) => {
                if let Some(i) = self.members.iter().position(|m| m.uid == u) {
                    self.remove_member_at(i);
                }
            }
            Sel::Gear(u) => {
                self.mech.gears.retain(|g| g.uid != u);
                self.mech.prune();
                self.refresh_reel_anchors();
            }
            Sel::Pulley(u) => {
                self.mech.pulleys.retain(|p| p.uid != u);
                self.refresh_reel_anchors();
            }
            Sel::Slider(u) => {
                self.mech.sliders.retain(|s| s.uid != u);
                self.mech.prune();
            }
            Sel::Pipe(u) => self.mech.plumbing.remove_pipe(u),
            Sel::Valve(u) => self.mech.plumbing.valves.retain(|v| v.uid != u),
            Sel::Pump(u) => self.mech.plumbing.pumps.retain(|p| p.uid != u),
            Sel::Cable(i) => {
                if i < self.cables.len() {
                    self.cables.remove(i);
                }
            }
            Sel::Car(id) => self.cars.retain(|c| c.id != id),
        }
        self.refresh_mask();
    }

    /// The selection outlined, with a handle on each end of a line-like element.
    pub fn draw_selection(&self, view: View) {
        let Some(sel) = self.selection else { return };
        let col = Color::new(1.0, 0.85, 0.3, 1.0);
        if let Some((a, b)) = self.sel_ends(sel) {
            let (sa, sb) = (view.to_screen(a), view.to_screen(b));
            draw_line(sa.x, sa.y, sb.x, sb.y, 5.0 * view.s.max(0.8), Color::new(1.0, 0.85, 0.3, 0.35));
            for p in [sa, sb] {
                draw_circle(p.x, p.y, (HANDLE * 0.7) * view.s.max(0.9), Color::new(0.07, 0.08, 0.15, 1.0));
                draw_circle_lines(p.x, p.y, (HANDLE * 0.7) * view.s.max(0.9), 2.4, col);
            }
            return;
        }
        let Some(c) = self.sel_centre(sel) else { return };
        let s = view.to_screen(c);
        match sel {
            Sel::Block(u) => {
                if let Some(b) = self.blocks.iter().find(|b| b.uid == u) {
                    let h = b.half();
                    if b.spec().round {
                        outline_circle(s, (h.x + 1.5) * view.s, col);
                    } else {
                        let pts: Vec<Vec2> = [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                            .iter()
                            .map(|(sx, sy)| view.to_screen(b.pos + sel_rot(b.angle, vec2(sx * (h.x + 1.5), sy * (h.y + 1.5)))))
                            .collect();
                        outline(&pts, col);
                    }
                }
            }
            Sel::Gear(u) => {
                if let Some(g) = self.mech.gears.iter().find(|g| g.uid == u) {
                    outline_circle(s, (g.size.radius() + 2.5) * view.s, col);
                }
            }
            Sel::Pulley(u) => {
                if let Some(p) = self.mech.pulleys.iter().find(|p| p.uid == u) {
                    outline_circle(s, (p.radius + 2.5) * view.s, col);
                }
            }
            Sel::Car(id) => {
                if let Some(c) = self.cars.iter().find(|c| c.id == id) {
                    outline_car(c, view, col);
                }
            }
            Sel::Valve(_) | Sel::Pump(_) => {
                // A little box around it, turned along its pipe.
                let pipe_uid = match sel {
                    Sel::Valve(u) => self.mech.plumbing.valves.iter().find(|v| v.uid == u).map(|v| v.pipe),
                    Sel::Pump(u) => self.mech.plumbing.pumps.iter().find(|p| p.uid == u).map(|p| p.pipe),
                    _ => None,
                };
                let dir = pipe_uid
                    .and_then(|pu| self.mech.plumbing.pipes.iter().find(|p| p.uid == pu))
                    .map(|p| vec2(p.b.0 - p.a.0, p.b.1 - p.a.1).normalize_or_zero())
                    .filter(|d| d.length() > 0.5)
                    .unwrap_or(vec2(1.0, 0.0));
                let (hx, hy) = (9.0f32, 7.0f32);
                let pts: Vec<Vec2> = [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                    .iter()
                    .map(|(sx, sy)| view.to_screen(c + dir * (sx * hx) + vec2(-dir.y, dir.x) * (sy * hy)))
                    .collect();
                outline(&pts, col);
            }
            _ => outline_circle(s, 8.0 * view.s, col),
        }
    }
}

fn sel_rot(a: f32, v: Vec2) -> Vec2 {
    let (sn, cs) = a.sin_cos();
    vec2(v.x * cs - v.y * sn, v.x * sn + v.y * cs)
}

/// A closed outline with a soft halo under it.
fn outline(pts: &[Vec2], col: Color) {
    let halo = Color::new(col.r, col.g, col.b, 0.28);
    for k in 0..pts.len() {
        let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
        draw_line(a.x, a.y, b.x, b.y, 6.0, halo);
    }
    for k in 0..pts.len() {
        let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
        draw_line(a.x, a.y, b.x, b.y, 2.4, col);
    }
}

/// The outline of something round (a wheel, a gear, a pulley).
fn outline_circle(c: Vec2, r: f32, col: Color) {
    draw_circle_lines(c.x, c.y, r, 6.0, Color::new(col.r, col.g, col.b, 0.28));
    draw_circle_lines(c.x, c.y, r, 2.4, col);
}

/// A car's outline: its body and each of its wheels.
pub fn outline_car(c: &car::Car, view: View, col: Color) {
    let m = c.kind.model();
    let (hl, top, bot) = (m.hull_half, m.hull_top, m.hull_bottom);
    let slab_top = top * 0.3;
    let p = |lx: f32, ly: f32| view.to_screen(c.p + sel_rot(c.angle, vec2(lx * c.dir, ly) * car::S));
    let body = [p(-hl - 1.0, bot + 1.0), p(hl + 1.0, bot + 1.0), p(hl + 1.0, slab_top - 1.0), p(hl * 0.5, top - 1.5), p(-hl * 0.7, top - 1.5), p(-hl - 1.0, slab_top - 1.0)];
    outline(&body, col);
    for (i, w) in c.wheels.iter().enumerate() {
        outline_circle(view.to_screen(w.pos), (c.wheel_r(i) + 1.0) * view.s, col);
    }
}

#[allow(dead_code)]
fn _unused(_: Vec2) -> Vec2 {
    snap_to_axes(vec2(1.0, 0.0), 1.0)
}
