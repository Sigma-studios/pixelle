//! Integer geometry on the canvas. `y` grows downwards, as in every image format.

use serde::{Deserialize, Serialize};

pub use glam::IVec2;

/// An axis-aligned rectangle of whole pixels: `x..x + w` by `y..y + h`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl IRect {
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    pub fn from_min_max(min: IVec2, max: IVec2) -> Self {
        Self::new(
            min.x,
            min.y,
            (max.x - min.x).max(0) as u32,
            (max.y - min.y).max(0) as u32,
        )
    }

    pub fn min(&self) -> IVec2 {
        IVec2::new(self.x, self.y)
    }

    /// One past the last pixel on each axis.
    pub fn max(&self) -> IVec2 {
        IVec2::new(self.x + self.w as i32, self.y + self.h as i32)
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    pub fn contains(&self, p: IVec2) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.max().x && p.y < self.max().y
    }

    pub fn union(&self, other: &IRect) -> IRect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        IRect::from_min_max(self.min().min(other.min()), self.max().max(other.max()))
    }

    pub fn intersect(&self, other: &IRect) -> IRect {
        IRect::from_min_max(self.min().max(other.min()), self.max().min(other.max()))
    }

    pub fn offset(&self, by: IVec2) -> IRect {
        IRect::new(self.x + by.x, self.y + by.y, self.w, self.h)
    }

    pub fn grow(&self, by: i32) -> IRect {
        IRect::from_min_max(self.min() - by, self.max() + by)
    }
}
