//! Zooming into the world and moving about it: Ctrl + wheel zooms on the pointer, Ctrl + drag moves
//! the view, Escape goes back to the standard view (the whole world fitted to the window).

use crate::render::View;
use crate::world::{H, W};
use macroquad::prelude::*;

/// The furthest one can zoom in, from the standard view.
pub const MAX_ZOOM: f32 = 8.0;
/// Each wheel notch zooms by this much.
const STEP: f32 = 1.25;

#[derive(Clone, Copy, Default)]
pub struct Camera {
    /// None: the standard view. Otherwise the zoom (from the standard view, 1 or more) and the
    /// world point at the top left corner of the area the world is shown in.
    look: Option<(f32, Vec2)>,
    /// While Ctrl + dragging: the world point that stays under the pointer.
    grab: Option<Vec2>,
}

impl Camera {
    pub fn is_standard(&self) -> bool {
        self.look.is_none()
    }

    pub fn zoom(&self) -> f32 {
        self.look.map_or(1.0, |(z, _)| z)
    }

    pub fn dragging(&self) -> bool {
        self.grab.is_some()
    }

    pub fn reset(&mut self) {
        *self = Camera::default();
    }

    /// The view seen through the camera, from the standard view `base`.
    pub fn apply(&self, base: View) -> View {
        let Some((z, o)) = self.look else { return base };
        let s = base.s * z;
        let o = clamp_origin(o, s, base.area);
        View { ox: base.area.x - o.x * s, oy: base.area.y - o.y * s, s, area: base.area }
    }

    /// Zooms by `notches` wheel notches (positive: in), keeping the world point under `mouse` still.
    pub fn zoom_at(&mut self, base: View, mouse: Vec2, notches: f32) {
        let view = self.apply(base);
        let at = view.to_world(mouse);
        let z = (self.zoom() * STEP.powf(notches)).clamp(1.0, MAX_ZOOM);
        if z <= 1.0 + 1e-3 {
            self.look = None;
            return;
        }
        let s = base.s * z;
        let o = at - (mouse - vec2(base.area.x, base.area.y)) / s;
        self.look = Some((z, clamp_origin(o, s, base.area)));
    }

    /// Ctrl + drag: `pressed` on the first frame of the button, `down` while it is held.
    pub fn drag(&mut self, base: View, mouse: Vec2, pressed: bool, down: bool) {
        if !down {
            self.grab = None;
            return;
        }
        let view = self.apply(base);
        if pressed || self.grab.is_none() {
            self.grab = Some(view.to_world(mouse));
            if self.look.is_none() {
                // Dragging the standard view still lets one see the parts the window crops.
                self.look = Some((1.0, view.to_world(vec2(base.area.x, base.area.y))));
            }
            return;
        }
        let (Some(at), Some((z, _))) = (self.grab, self.look) else { return };
        let s = base.s * z;
        let o = at - (mouse - vec2(base.area.x, base.area.y)) / s;
        self.look = Some((z, clamp_origin(o, s, base.area)));
    }
}

/// Keeps the world covering the whole area (no empty band at the edges).
fn clamp_origin(o: Vec2, s: f32, area: Rect) -> Vec2 {
    let (vw, vh) = (area.w / s, area.h / s);
    vec2(o.x.clamp(0.0, (W as f32 - vw).max(0.0)), o.y.clamp(0.0, (H as f32 - vh).max(0.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> View {
        View::fit(Rect::new(0.0, 40.0, 1200.0, 700.0))
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer_and_escape_comes_back() {
        let mut c = Camera::default();
        let mouse = vec2(500.0, 300.0);
        let before = c.apply(base()).to_world(mouse);
        c.zoom_at(base(), mouse, 3.0);
        assert!(!c.is_standard());
        assert!((c.zoom() - STEP.powi(3)).abs() < 1e-4);
        let after = c.apply(base()).to_world(mouse);
        assert!(before.distance(after) < 1e-3, "{before} -> {after}");
        assert!((c.apply(base()).s - base().s * c.zoom()).abs() < 1e-4);
        c.reset();
        assert!(c.is_standard());
        assert_eq!(c.apply(base()).ox, base().ox);
    }

    #[test]
    fn zoom_is_bounded_and_zooming_back_out_returns_to_the_standard_view() {
        let mut c = Camera::default();
        c.zoom_at(base(), vec2(600.0, 400.0), 100.0);
        assert_eq!(c.zoom(), MAX_ZOOM);
        c.zoom_at(base(), vec2(600.0, 400.0), -100.0);
        assert!(c.is_standard());
        c.zoom_at(base(), vec2(600.0, 400.0), -1.0);
        assert!(c.is_standard(), "no zooming out past the whole world");
    }

    #[test]
    fn dragging_moves_the_world_with_the_pointer_but_never_off_the_window() {
        let mut c = Camera::default();
        c.zoom_at(base(), vec2(600.0, 400.0), 4.0);
        let grip = c.apply(base()).to_world(vec2(600.0, 400.0));
        c.drag(base(), vec2(600.0, 400.0), true, true);
        c.drag(base(), vec2(560.0, 380.0), false, true);
        let moved = c.apply(base()).to_world(vec2(560.0, 380.0));
        assert!(grip.distance(moved) < 1e-3, "the grabbed point follows the pointer");
        c.drag(base(), vec2(560.0, 380.0), false, false);
        assert!(!c.dragging());
        // A huge drag stops at the world's edge.
        c.drag(base(), vec2(0.0, 0.0), true, true);
        c.drag(base(), vec2(100_000.0, 100_000.0), false, true);
        let v = c.apply(base());
        assert!(v.ox <= v.area.x + 1e-3 && v.oy <= v.area.y + 1e-3);
        let r = v.rect();
        assert!(r.x + r.w >= v.area.x + v.area.w - 1e-3 && r.y + r.h >= v.area.y + v.area.h - 1e-3);
    }
}
