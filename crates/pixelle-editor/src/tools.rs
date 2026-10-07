//! The drawing tools and the pixel geometry behind them.

use std::collections::{HashSet, VecDeque};

use pixelle_core::{IRect, IVec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pencil,
    Eraser,
    Line,
    Rect,
    Fill,
    Picker,
    Select,
    Terrain,
    Sprite,
    Hand,
}

impl Tool {
    pub const ALL: [Tool; 10] = [
        Tool::Pencil,
        Tool::Eraser,
        Tool::Line,
        Tool::Rect,
        Tool::Fill,
        Tool::Picker,
        Tool::Select,
        Tool::Terrain,
        Tool::Sprite,
        Tool::Hand,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tool::Pencil => "Pencil",
            Tool::Eraser => "Eraser",
            Tool::Line => "Line",
            Tool::Rect => "Rectangle",
            Tool::Fill => "Fill",
            Tool::Picker => "Picker",
            Tool::Select => "Select",
            Tool::Terrain => "Terrain",
            Tool::Sprite => "Frames",
            Tool::Hand => "Hand",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Tool::Pencil => "B",
            Tool::Eraser => "E",
            Tool::Line => "L",
            Tool::Rect => "R",
            Tool::Fill => "G",
            Tool::Picker => "I",
            Tool::Select => "M",
            Tool::Terrain => "T",
            Tool::Sprite => "S",
            Tool::Hand => "H",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Tool::Pencil => "Left: primary · Right: secondary · Alt: pick",
            Tool::Eraser => "Drag to erase on the active layer",
            Tool::Line => "Drag a line · Right: secondary",
            Tool::Rect => "Drag a rectangle · Shift: filled",
            Tool::Fill => "Fill the region under the cursor (open space only inside a selection) · Shift: dither",
            Tool::Picker => "Left: primary · Right: secondary",
            Tool::Select => "Drag to select · drag inside to move · ⌘C/⌘X/⌘V · arrows nudge · Enter commits",
            Tool::Terrain => "Paint the active terrain on a terrain layer · Right: erase",
            Tool::Sprite => "Click a frame to select it · drag its corner to resize · Alt: off-grid",
            Tool::Hand => "Drag to pan (Space or the middle button pan with any tool)",
        }
    }

    /// Tools that draw on a pixel layer.
    pub fn paints(self) -> bool {
        matches!(self, Tool::Pencil | Tool::Eraser | Tool::Line | Tool::Rect | Tool::Fill)
    }
}

/// Every pixel from `a` to `b`, both included, with no gaps and no doubled corners.
pub fn line(a: IVec2, b: IVec2) -> Vec<IVec2> {
    let d = (b - a).abs();
    let s = IVec2::new((b.x - a.x).signum(), (b.y - a.y).signum());
    let mut err = d.x - d.y;
    let mut p = a;
    let mut out = Vec::with_capacity((d.x.max(d.y) + 1) as usize);
    loop {
        out.push(p);
        if p == b {
            return out;
        }
        let e2 = 2 * err;
        if e2 > -d.y {
            err -= d.y;
            p.x += s.x;
        }
        if e2 < d.x {
            err += d.x;
            p.y += s.y;
        }
    }
}

/// The rectangle with corners `a` and `b`, both included.
pub fn span(a: IVec2, b: IVec2) -> IRect {
    IRect::from_min_max(a.min(b), a.max(b) + IVec2::ONE)
}

pub fn rect_pixels(r: IRect, filled: bool) -> Vec<IVec2> {
    let mut out = Vec::new();
    for y in r.y..r.max().y {
        for x in r.x..r.max().x {
            let edge = x == r.x || y == r.y || x == r.max().x - 1 || y == r.max().y - 1;
            if filled || edge {
                out.push(IVec2::new(x, y));
            }
        }
    }
    out
}

/// The 4-connected region of pixels equal to the one at `start`, inside `bounds`.
pub fn flood(start: IVec2, bounds: IRect, get: impl Fn(IVec2) -> u8) -> Vec<IVec2> {
    if !bounds.contains(start) {
        return Vec::new();
    }
    let target = get(start);
    let mut seen = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    let mut out = Vec::new();
    while let Some(p) = queue.pop_front() {
        out.push(p);
        for d in [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
            let n = p + d;
            if bounds.contains(n) && !seen.contains(&n) && get(n) == target {
                seen.insert(n);
                queue.push_back(n);
            }
        }
    }
    out
}

/// Whether `b` is the elbow of an L between `a` and `c` — the pixel "pixel-perfect" drawing
/// removes so a freehand diagonal stays one pixel thin.
pub fn is_elbow(a: IVec2, b: IVec2, c: IVec2) -> bool {
    let adjacent = |p: IVec2, q: IVec2| (p - q).abs().element_sum() == 1;
    adjacent(a, b) && adjacent(b, c) && (a - c).abs() == IVec2::ONE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_have_no_gaps() {
        let l = line(IVec2::new(0, 0), IVec2::new(5, 2));
        assert_eq!(l.first(), Some(&IVec2::new(0, 0)));
        assert_eq!(l.last(), Some(&IVec2::new(5, 2)));
        for w in l.windows(2) {
            assert!((w[1] - w[0]).abs().max_element() == 1);
        }
        assert_eq!(line(IVec2::ONE, IVec2::ONE), vec![IVec2::ONE]);
    }

    #[test]
    fn flood_stays_in_its_region_and_bounds() {
        // A wall down x = 2.
        let get = |p: IVec2| u8::from(p.x == 2);
        let region = flood(IVec2::ZERO, IRect::new(0, 0, 5, 3), get);
        assert_eq!(region.len(), 6);
        assert!(region.iter().all(|p| p.x < 2));
    }

    #[test]
    fn elbows_are_found() {
        assert!(is_elbow(IVec2::new(0, 0), IVec2::new(1, 0), IVec2::new(1, 1)));
        assert!(!is_elbow(IVec2::new(0, 0), IVec2::new(1, 0), IVec2::new(2, 0)));
    }

    #[test]
    fn rectangles() {
        assert_eq!(rect_pixels(span(IVec2::new(2, 2), IVec2::ZERO), false).len(), 8);
        assert_eq!(rect_pixels(IRect::new(0, 0, 3, 3), true).len(), 9);
    }
}
