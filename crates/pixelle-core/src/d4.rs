//! The eight orientations of a square: the dihedral group D4.
//!
//! A tile drawn once can stand in for up to eight cells — a corner drawn at the top left is also
//! every other corner — as long as the renderer can transpose as well as flip. Keeping the
//! orientation as data rather than baking every variant is what lets a terrain be authored from a
//! handful of canonical tiles.

use serde::{Deserialize, Serialize};

use crate::geom::IVec2;

/// Applied in the order transpose, then flip `x`, then flip `y`.
///
/// So `D4 { transpose: true, flip_x: true, .. }` is "transpose the picture, then mirror the result
/// left to right" — which is exactly how a sprite with `flip_x` set draws a pre-transposed frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct D4 {
    pub transpose: bool,
    pub flip_x: bool,
    pub flip_y: bool,
}

impl D4 {
    pub const IDENTITY: D4 = D4::new(false, false, false);
    pub const TRANSPOSE: D4 = D4::new(true, false, false);
    pub const FLIP_X: D4 = D4::new(false, true, false);
    pub const FLIP_Y: D4 = D4::new(false, false, true);

    pub const fn new(transpose: bool, flip_x: bool, flip_y: bool) -> Self {
        Self {
            transpose,
            flip_x,
            flip_y,
        }
    }

    /// All eight, identity first. The order is part of the contract: a symmetric rule tries them
    /// in this order and the first match wins.
    pub const ALL: [D4; 8] = [
        D4::new(false, false, false),
        D4::new(false, true, false),
        D4::new(false, false, true),
        D4::new(false, true, true),
        D4::new(true, false, false),
        D4::new(true, true, false),
        D4::new(true, false, true),
        D4::new(true, true, true),
    ];

    pub fn is_identity(&self) -> bool {
        *self == D4::IDENTITY
    }

    /// Where the pixel at `p` of a `w`×`h` picture lands. A transposing orientation swaps the
    /// picture's width and height, see [`D4::size`].
    pub fn apply(&self, p: IVec2, w: u32, h: u32) -> IVec2 {
        let (mut x, mut y, ow, oh) = if self.transpose {
            (p.y, p.x, h as i32, w as i32)
        } else {
            (p.x, p.y, w as i32, h as i32)
        };
        if self.flip_x {
            x = ow - 1 - x;
        }
        if self.flip_y {
            y = oh - 1 - y;
        }
        IVec2::new(x, y)
    }

    /// The size of a `w`×`h` picture after this orientation.
    pub fn size(&self, w: u32, h: u32) -> (u32, u32) {
        if self.transpose { (h, w) } else { (w, h) }
    }

    /// Where a neighbour offset (each axis in -1..=1) lands.
    pub fn apply_offset(&self, d: IVec2) -> IVec2 {
        self.apply(d + IVec2::ONE, 3, 3) - IVec2::ONE
    }

    /// `self` after `first`: applying the result is applying `first` and then `self`.
    pub fn after(&self, first: D4) -> D4 {
        // Two asymmetric probe points pin down an element of D4 uniquely.
        let probes = [IVec2::new(0, 1), IVec2::new(2, 1), IVec2::new(1, 0)];
        let want: Vec<IVec2> = probes
            .iter()
            .map(|p| self.apply(first.apply(*p, 3, 3), 3, 3))
            .collect();
        *D4::ALL
            .iter()
            .find(|g| probes.iter().zip(&want).all(|(p, w)| g.apply(*p, 3, 3) == *w))
            .expect("D4 is closed under composition")
    }

    /// The orientation that undoes this one.
    pub fn inverse(&self) -> D4 {
        *D4::ALL
            .iter()
            .find(|g| g.after(*self).is_identity())
            .expect("every element of D4 has an inverse")
    }
}

/// Compact: `id`, or the steps in order, e.g. `transpose+flip_x`.
impl std::fmt::Display for D4 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let steps: Vec<&str> = [
            (self.transpose, "transpose"),
            (self.flip_x, "flip_x"),
            (self.flip_y, "flip_y"),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, s)| *s)
        .collect();
        if steps.is_empty() {
            write!(f, "id")
        } else {
            write!(f, "{}", steps.join("+"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_is_closed_and_invertible() {
        for a in D4::ALL {
            assert!(a.inverse().after(a).is_identity());
            for b in D4::ALL {
                let c = a.after(b);
                for y in 0..3 {
                    for x in 0..3 {
                        let p = IVec2::new(x, y);
                        assert_eq!(c.apply(p, 3, 3), a.apply(b.apply(p, 3, 3), 3, 3));
                    }
                }
            }
        }
    }

    #[test]
    fn transpose_swaps_axes() {
        assert_eq!(D4::TRANSPOSE.apply(IVec2::new(3, 1), 8, 8), IVec2::new(1, 3));
        assert_eq!(D4::TRANSPOSE.size(3, 1), (1, 3));
    }

    #[test]
    fn flip_mirrors_within_the_picture() {
        assert_eq!(D4::FLIP_X.apply(IVec2::new(0, 2), 8, 8), IVec2::new(7, 2));
        assert_eq!(D4::FLIP_Y.apply_offset(IVec2::new(1, -1)), IVec2::new(1, 1));
    }
}
