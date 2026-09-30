//! Where the canvas is on screen: a centre and a whole-number zoom.

use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};
use pixelle_core::{IRect, IVec2};

/// The zoom steps, in screen points per canvas pixel. Whole numbers only, so every canvas pixel is
/// the same size on screen.
pub const ZOOMS: [f32; 13] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 16.0, 24.0, 32.0, 48.0];

#[derive(Clone, Copy, Debug)]
pub struct View {
    /// The canvas point at the middle of the canvas area.
    pub center: Vec2,
    pub zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            center: Vec2::ZERO,
            zoom: 4.0,
        }
    }
}

impl View {
    /// Where canvas (0, 0) lands, rounded so pixel edges fall on whole screen points.
    pub fn origin(&self, screen: Rect) -> Pos2 {
        let o = screen.center() - self.center * self.zoom;
        pos2(o.x.round(), o.y.round())
    }

    pub fn to_screen(&self, screen: Rect, canvas: Pos2) -> Pos2 {
        self.origin(screen) + canvas.to_vec2() * self.zoom
    }

    pub fn to_canvas(&self, screen: Rect, s: Pos2) -> Pos2 {
        ((s - self.origin(screen)) / self.zoom).to_pos2()
    }

    /// The canvas pixel under a screen point.
    pub fn pixel(&self, screen: Rect, s: Pos2) -> IVec2 {
        let c = self.to_canvas(screen, s);
        IVec2::new(c.x.floor() as i32, c.y.floor() as i32)
    }

    pub fn rect_to_screen(&self, screen: Rect, r: IRect) -> Rect {
        Rect::from_min_max(
            self.to_screen(screen, pos2(r.x as f32, r.y as f32)),
            self.to_screen(screen, pos2(r.max().x as f32, r.max().y as f32)),
        )
    }

    /// The canvas pixels at least partly on screen.
    pub fn visible(&self, screen: Rect) -> IRect {
        let a = self.to_canvas(screen, screen.min);
        let b = self.to_canvas(screen, screen.max);
        IRect::from_min_max(
            IVec2::new(a.x.floor() as i32, a.y.floor() as i32),
            IVec2::new(b.x.ceil() as i32, b.y.ceil() as i32),
        )
    }

    /// Change zoom keeping the canvas point under `anchor` where it is.
    pub fn zoom_to(&mut self, screen: Rect, anchor: Pos2, zoom: f32) {
        let before = self.to_canvas(screen, anchor);
        self.zoom = zoom;
        let after = self.to_canvas(screen, anchor);
        self.center += before - after;
    }

    pub fn step_zoom(&mut self, screen: Rect, anchor: Pos2, steps: i32) {
        let i = ZOOMS.iter().position(|z| *z >= self.zoom).unwrap_or(ZOOMS.len() - 1) as i32;
        let next = ZOOMS[(i + steps).clamp(0, ZOOMS.len() as i32 - 1) as usize];
        self.zoom_to(screen, anchor, next);
    }

    /// Show all of `r`, as large as fits.
    pub fn fit(&mut self, screen: Rect, r: IRect) {
        if r.is_empty() {
            return;
        }
        let fits = |z: f32| r.w as f32 * z <= screen.width() * 0.92 && r.h as f32 * z <= screen.height() * 0.92;
        self.zoom = ZOOMS.iter().rev().copied().find(|z| fits(*z)).unwrap_or(1.0);
        self.center = vec2(r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0);
    }

    pub fn pan_by_screen(&mut self, delta: Vec2) {
        self.center -= delta / self.zoom;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_and_canvas_agree() {
        let screen = Rect::from_min_size(pos2(10.0, 20.0), vec2(400.0, 300.0));
        let mut v = View { center: vec2(5.5, -3.0), zoom: 8.0 };
        let p = v.pixel(screen, v.to_screen(screen, pos2(12.5, 7.5)));
        assert_eq!(p, IVec2::new(12, 7));
        let anchor = pos2(100.0, 100.0);
        let under = v.to_canvas(screen, anchor);
        v.zoom_to(screen, anchor, 16.0);
        assert!((v.to_canvas(screen, anchor) - under).length() < 0.1);
    }
}
