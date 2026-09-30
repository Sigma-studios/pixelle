//! Tilesets and terrains.
//!
//! A [`Tileset`] is a grid of tiles cut from the canvas itself, so drawing on a tile is drawing on
//! every cell that shows it. A [`Terrain`] is a list of [`Rule`]s that pick a tile, and the
//! orientation to draw it in, for a cell from which of its eight neighbours are the same terrain.

use serde::{Deserialize, Serialize};

use crate::d4::D4;
use crate::geom::{IRect, IVec2};
use crate::layer::LayerId;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tileset {
    pub name: String,
    /// The top-left pixel of tile 0.
    pub origin: IVec2,
    pub tile_size: u32,
    pub columns: u32,
    pub count: u32,
    /// The layers the tiles are drawn on.
    pub layers: Vec<LayerId>,
}

impl Tileset {
    pub fn tile_rect(&self, index: u32) -> IRect {
        let (col, row) = (index % self.columns, index / self.columns);
        IRect::new(
            self.origin.x + (col * self.tile_size) as i32,
            self.origin.y + (row * self.tile_size) as i32,
            self.tile_size,
            self.tile_size,
        )
    }

    pub fn bounds(&self) -> IRect {
        let rows = self.count.div_ceil(self.columns);
        IRect::new(
            self.origin.x,
            self.origin.y,
            self.columns * self.tile_size,
            rows * self.tile_size,
        )
    }
}

/// What a rule asks of one neighbour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cond {
    Any,
    /// The same terrain.
    Same,
    /// Anything else: empty, or another terrain.
    NotSame,
}

/// Neighbour offsets, in the order a rule's pattern lists them: the row above left to right, the
/// two beside, then the row below.
pub const NEIGHBOURS: [IVec2; 8] = [
    IVec2::new(-1, -1),
    IVec2::new(0, -1),
    IVec2::new(1, -1),
    IVec2::new(-1, 0),
    IVec2::new(1, 0),
    IVec2::new(-1, 1),
    IVec2::new(0, 1),
    IVec2::new(1, 1),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    /// What the tile is, for people: "corner", "inside".
    pub name: String,
    pub pattern: [Cond; 8],
    pub tile: u32,
    #[serde(default)]
    pub transform: D4,
    /// Also match the pattern in each of the eight orientations, drawing the tile turned to match.
    #[serde(default)]
    pub symmetric: bool,
}

impl Rule {
    /// A rule from a 3×3 picture of its neighbourhood, rows separated by whitespace: `#` is the
    /// same terrain, `.` is anything else, `?` is either, and the middle is ignored.
    ///
    /// ```
    /// # use pixelle_core::tiles::Rule;
    /// let top = Rule::parse("top", "?.? ##? ?#?", 1).unwrap();
    /// ```
    pub fn parse(name: &str, picture: &str, tile: u32) -> Result<Rule, String> {
        let cells: Vec<char> = picture.chars().filter(|c| !c.is_whitespace()).collect();
        if cells.len() != 9 {
            return Err(format!("rule {name:?}: a pattern is 9 cells, got {}", cells.len()));
        }
        let mut pattern = [Cond::Any; 8];
        let mut n = 0;
        for (i, c) in cells.iter().enumerate() {
            if i == 4 {
                continue;
            }
            pattern[n] = match c {
                '#' => Cond::Same,
                '.' => Cond::NotSame,
                '?' => Cond::Any,
                other => return Err(format!("rule {name:?}: unknown cell {other:?}")),
            };
            n += 1;
        }
        Ok(Rule {
            name: name.into(),
            pattern,
            tile,
            transform: D4::IDENTITY,
            symmetric: false,
        })
    }

    pub fn symmetric(mut self) -> Self {
        self.symmetric = true;
        self
    }

    pub fn with_transform(mut self, t: D4) -> Self {
        self.transform = t;
        self
    }

    /// The tile and orientation this rule draws, if it matches a cell whose neighbours are the
    /// same terrain where `same` says so.
    pub fn resolve(&self, same: &impl Fn(IVec2) -> bool) -> Option<(u32, D4)> {
        let orientations: &[D4] = if self.symmetric {
            &D4::ALL
        } else {
            &[D4::IDENTITY]
        };
        orientations.iter().find_map(|g| {
            let matches = NEIGHBOURS.iter().zip(self.pattern).all(|(offset, cond)| {
                let is_same = same(g.apply_offset(*offset));
                match cond {
                    Cond::Any => true,
                    Cond::Same => is_same,
                    Cond::NotSame => !is_same,
                }
            });
            matches.then(|| (self.tile, g.after(self.transform)))
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Terrain {
    pub name: String,
    /// The tileset's name.
    pub tileset: String,
    /// Tried in order; the first that matches wins.
    pub rules: Vec<Rule>,
    /// Drawn where no rule matches.
    pub fallback: Option<u32>,
}

impl Terrain {
    /// The tile a cell shows, given which of its neighbours (by offset) are the same terrain.
    pub fn resolve(&self, same: impl Fn(IVec2) -> bool) -> Option<(u32, D4)> {
        self.rules
            .iter()
            .find_map(|rule| rule.resolve(&same))
            .or(self.fallback.map(|tile| (tile, D4::IDENTITY)))
    }
}

/// Ready-made rule sets.
pub mod presets {
    use super::*;

    /// Which tile of a tileset plays each part in [`minimal_d4`]. Every tile is drawn with its
    /// outline at the *start* of each axis it is outlined on — the left column, the top row.
    #[derive(Clone, Copy, Debug)]
    pub struct MinimalD4 {
        /// Outline left and top.
        pub corner: u32,
        /// Outline along the top.
        pub edge: u32,
        /// Outline left, right and top: the end of a strip one tile wide.
        pub strip_end: u32,
        /// Outline left and right: the middle of a strip one tile wide.
        pub strip: u32,
        /// No outline.
        pub interior: u32,
        /// Surrounded on all four sides, open on the diagonal above right: the one lit pixel of a
        /// concave corner.
        pub inside: u32,
        /// Surrounded on all four sides, open on both diagonals above: the foot of a one-wide
        /// strip standing on a surface.
        pub double_inside: u32,
    }

    /// Autotiling from seven canonical tiles and the eight orientations of a square.
    ///
    /// The set is complete for any shape made of whole cells; a lone cell, which no tile draws,
    /// shows the strip end.
    pub fn minimal_d4(t: MinimalD4) -> Vec<Rule> {
        let rule = |name: &str, picture: &str, tile: u32| {
            Rule::parse(name, picture, tile)
                .expect("preset patterns are well-formed")
                .symmetric()
        };
        vec![
            rule("single", "?.? .x. ?.?", t.strip_end),
            rule("strip end", "?.? .x. ?#?", t.strip_end),
            rule("strip", "?#? .x. ?#?", t.strip),
            rule("corner", "?.? .x# ?#?", t.corner),
            rule("edge", "?.? #x# ?#?", t.edge),
            rule("double inside", ".#. #x# ?#?", t.double_inside),
            rule("inside", "##. #x# ?#?", t.inside),
            rule("interior", "?#? #x# ?#?", t.interior),
        ]
    }

    /// A horizontal run: caps at each end, a middle between. A run one cell long shows `single`.
    pub fn horizontal_run(left: u32, middle: u32, right: u32, single: u32) -> Vec<Rule> {
        let rule = |name: &str, picture: &str, tile: u32| {
            Rule::parse(name, picture, tile).expect("preset patterns are well-formed")
        };
        vec![
            rule("single", "??? .x. ???", single),
            rule("left cap", "??? .x# ???", left),
            rule("right cap", "??? #x. ???", right),
            rule("middle", "??? #x# ???", middle),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::presets::*;
    use super::*;
    use std::collections::HashSet;

    const CORNER: u32 = 0;
    const TOP: u32 = 1;
    const HEAD: u32 = 2;
    const BODY: u32 = 3;
    const INTERIOR: u32 = 4;
    const INSIDE: u32 = 5;
    const DOUBLE: u32 = 6;

    fn terrain() -> Terrain {
        Terrain {
            name: "walls".into(),
            tileset: "walls".into(),
            rules: minimal_d4(MinimalD4 {
                corner: CORNER,
                edge: TOP,
                strip_end: HEAD,
                strip: BODY,
                interior: INTERIOR,
                inside: INSIDE,
                double_inside: DOUBLE,
            }),
            fallback: Some(INTERIOR),
        }
    }

    /// Resolve every cell of an ASCII map, `#` solid.
    fn resolve(map: &str) -> Vec<Vec<Option<(u32, D4)>>> {
        let rows: Vec<&str> = map.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        let solid: HashSet<IVec2> = rows
            .iter()
            .enumerate()
            .flat_map(|(y, row)| {
                row.chars()
                    .enumerate()
                    .filter(|(_, c)| *c == '#')
                    .map(move |(x, _)| IVec2::new(x as i32, y as i32))
            })
            .collect();
        let t = terrain();
        rows.iter()
            .enumerate()
            .map(|(y, row)| {
                (0..row.len())
                    .map(|x| {
                        let at = IVec2::new(x as i32, y as i32);
                        solid
                            .contains(&at)
                            .then(|| t.resolve(|d| solid.contains(&(at + d))).unwrap())
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_block_has_corners_edges_and_an_interior() {
        let r = resolve(
            "###
             ###
             ###",
        );
        assert_eq!(r[0][0], Some((CORNER, D4::IDENTITY)));
        assert_eq!(r[0][2], Some((CORNER, D4::FLIP_X)));
        assert_eq!(r[2][0], Some((CORNER, D4::FLIP_Y)));
        assert_eq!(r[2][2], Some((CORNER, D4::new(false, true, true))));
        assert_eq!(r[0][1], Some((TOP, D4::IDENTITY)));
        assert_eq!(r[2][1], Some((TOP, D4::FLIP_Y)));
        // The left edge is the top edge transposed, and the right edge that mirrored.
        assert_eq!(r[1][0], Some((TOP, D4::TRANSPOSE)));
        assert_eq!(r[1][2], Some((TOP, D4::new(true, true, false))));
        assert_eq!(r[1][1], Some((INTERIOR, D4::IDENTITY)));
    }

    #[test]
    fn narrow_shapes_are_strips() {
        let column = resolve(
            "#
             #
             #",
        );
        assert_eq!(column[0][0], Some((HEAD, D4::IDENTITY)));
        assert_eq!(column[1][0], Some((BODY, D4::IDENTITY)));
        assert_eq!(column[2][0], Some((HEAD, D4::FLIP_Y)));

        let bar = resolve("###");
        assert_eq!(bar[0][0], Some((HEAD, D4::TRANSPOSE)));
        assert_eq!(bar[0][1], Some((BODY, D4::TRANSPOSE)));
        assert_eq!(bar[0][2], Some((HEAD, D4::new(true, true, false))));
    }

    #[test]
    fn a_pillar_on_a_floor_turns_the_grass_up_into_it() {
        let r = resolve(
            ".#..
             .#..
             ####
             ####",
        );
        // The floor under a one-wide pillar is open on both upper diagonals.
        assert_eq!(r[2][1], Some((DOUBLE, D4::IDENTITY)));
        // The pillar's foot is a strip running into the floor.
        assert_eq!(r[1][1], Some((BODY, D4::IDENTITY)));

        let wide = resolve(
            ".##.
             ####
             ####",
        );
        // Under the right end of a wide pillar the open diagonal is above right: `inside` as drawn.
        assert_eq!(wide[1][2], Some((INSIDE, D4::IDENTITY)));
        // And under the left end, mirrored.
        assert_eq!(wide[1][1], Some((INSIDE, D4::FLIP_X)));
    }

    #[test]
    fn runs_have_caps() {
        let rules = horizontal_run(0, 1, 2, 1);
        let t = Terrain {
            name: "p".into(),
            tileset: "p".into(),
            rules,
            fallback: None,
        };
        let solid = [0, 1, 2];
        let at = |x: i32| t.resolve(|d| d.y == 0 && solid.contains(&(x + d.x))).unwrap().0;
        assert_eq!((at(0), at(1), at(2)), (0, 1, 2));
    }
}
