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

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Terrain {
    pub name: String,
    /// The tileset's name.
    pub tileset: String,
    /// Tried in order; the first that matches wins.
    pub rules: Vec<Rule>,
    /// Drawn where no rule matches.
    pub fallback: Option<u32>,
    /// Where no rule matches, draw the cell in four quarters instead, each chosen by these from
    /// its own corner of the neighbourhood. Empty: no quarters. Tried before `fallback`.
    pub quarters: Vec<QuarterRule>,
}

/// How to draw one quarter of a cell, written for the top-left quarter; the other three are its
/// mirror images.
///
/// A quarter looks at three neighbours: the one above it (`vertical`), the one beside it
/// (`horizontal`) and the one off its corner (`diagonal`). When they match, it shows the top-left
/// quarter of `tile` in orientation `transform`, mirrored into place.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuarterRule {
    pub name: String,
    pub vertical: Cond,
    pub horizontal: Cond,
    pub diagonal: Cond,
    pub tile: u32,
    pub transform: D4,
}

/// What a cell shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Whole { tile: u32, transform: D4 },
    /// Top-left, top-right, bottom-left, bottom-right.
    Quarters([Quarter; 4]),
}

/// One quarter of a cell: the top-left quarter of `tile` in orientation `transform`, then
/// flipped by `place` into its corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quarter {
    pub tile: u32,
    pub transform: D4,
    pub place: D4,
}

/// The flips that carry the top-left quarter into each corner, in [`Choice::Quarters`] order.
pub const QUARTER_PLACES: [D4; 4] = [
    D4::new(false, false, false),
    D4::new(false, true, false),
    D4::new(false, false, true),
    D4::new(false, true, true),
];

impl QuarterRule {
    fn matches(&self, place: D4, same: &impl Fn(IVec2) -> bool) -> bool {
        let test = |cond: Cond, d: IVec2| match cond {
            Cond::Any => true,
            Cond::Same => same(place.apply_offset(d)),
            Cond::NotSame => !same(place.apply_offset(d)),
        };
        test(self.vertical, IVec2::new(0, -1))
            && test(self.horizontal, IVec2::new(-1, 0))
            && test(self.diagonal, IVec2::new(-1, -1))
    }
}

impl Terrain {
    pub fn new(name: impl Into<String>, tileset: impl Into<String>, rules: Vec<Rule>) -> Self {
        Self {
            name: name.into(),
            tileset: tileset.into(),
            rules,
            fallback: None,
            quarters: Vec::new(),
        }
    }

    /// The whole tile a cell shows, given which of its neighbours (by offset) are the same
    /// terrain — ignoring quarters.
    pub fn resolve(&self, same: impl Fn(IVec2) -> bool) -> Option<(u32, D4)> {
        self.rules
            .iter()
            .find_map(|rule| rule.resolve(&same))
            .or(self.fallback.map(|tile| (tile, D4::IDENTITY)))
    }

    /// What a cell shows: a whole tile by the rules, else quarters, else the fallback.
    pub fn choose(&self, same: impl Fn(IVec2) -> bool) -> Option<Choice> {
        if let Some((tile, transform)) = self.rules.iter().find_map(|rule| rule.resolve(&same)) {
            return Some(Choice::Whole { tile, transform });
        }
        if !self.quarters.is_empty() {
            let mut out = [Quarter { tile: 0, transform: D4::IDENTITY, place: D4::IDENTITY }; 4];
            let mut all = true;
            for (q, place) in out.iter_mut().zip(QUARTER_PLACES) {
                match self.quarters.iter().find(|r| r.matches(place, &same)) {
                    Some(r) => *q = Quarter { tile: r.tile, transform: r.transform, place },
                    None => all = false,
                }
            }
            if all {
                return Some(Choice::Quarters(out));
            }
        }
        self.fallback.map(|tile| Choice::Whole { tile, transform: D4::IDENTITY })
    }

    /// Every orientation a tile may be drawn in, by any rule: what an engine that cannot
    /// transpose needs baked.
    pub fn needs_transpose(&self) -> bool {
        self.rules.iter().any(|r| r.symmetric || r.transform.transpose)
            || self.quarters.iter().any(|q| q.transform.transpose)
    }
}

/// Terrains were written without `quarters` before format version 2; the reader says which
/// layout it is reading through this.
pub(crate) mod compat {
    use std::cell::Cell;

    thread_local! {
        pub static FORMAT: Cell<u16> = const { Cell::new(crate::format::VERSION) };
    }
}

impl<'de> Deserialize<'de> for Terrain {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct V1 {
            name: String,
            tileset: String,
            rules: Vec<Rule>,
            fallback: Option<u32>,
        }
        #[derive(Deserialize)]
        struct V2 {
            name: String,
            tileset: String,
            rules: Vec<Rule>,
            fallback: Option<u32>,
            quarters: Vec<QuarterRule>,
        }
        if compat::FORMAT.with(|f| f.get()) < 2 {
            let t = V1::deserialize(d)?;
            Ok(Terrain { name: t.name, tileset: t.tileset, rules: t.rules, fallback: t.fallback, quarters: Vec::new() })
        } else {
            let t = V2::deserialize(d)?;
            Ok(Terrain { name: t.name, tileset: t.tileset, rules: t.rules, fallback: t.fallback, quarters: t.quarters })
        }
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
    /// A whole tile is used only where it is exactly right — an edge whose diagonals agree with
    /// it, a lone inside corner, a pair of inside corners above or below. Everything else is left
    /// to [`quarters_d4`], which draws any neighbourhood from the same seven tiles. This is how
    /// run-2d clads its grid maps (`cladding.rs`, `whole_frame` and `quarter`).
    pub fn minimal_d4(t: MinimalD4) -> Vec<Rule> {
        let rule = |name: &str, picture: &str, tile: u32| {
            Rule::parse(name, picture, tile).expect("preset patterns are well-formed")
        };
        vec![
            rule("interior", "### #x# ###", t.interior),
            rule("inside", "##. #x# ###", t.inside).symmetric(),
            rule("double inside", ".#. #x# ###", t.double_inside),
            rule("double inside below", "### #x# .#.", t.double_inside).with_transform(D4::FLIP_Y),
            rule("edge", "?.? #x# ###", t.edge).symmetric(),
            rule("corner", "?.? .x# ?##", t.corner).symmetric(),
            rule("strip", "?#? .x. ?#?", t.strip).symmetric(),
            rule("strip end", "?.? .x. ?#?", t.strip_end).symmetric(),
        ]
    }

    /// Quarters for [`minimal_d4`]: any neighbourhood, a corner at a time.
    pub fn quarters_d4(t: MinimalD4) -> Vec<QuarterRule> {
        let q = |name: &str, vertical, horizontal, diagonal, tile, transform| QuarterRule {
            name: name.into(),
            vertical,
            horizontal,
            diagonal,
            tile,
            transform,
        };
        use Cond::*;
        vec![
            q("corner", NotSame, NotSame, Any, t.corner, D4::IDENTITY),
            q("top", NotSame, Same, Any, t.edge, D4::IDENTITY),
            q("side", Same, NotSame, Any, t.edge, D4::TRANSPOSE),
            // The inside tile is lit at its top right; its right-hand quarter, mirrored.
            q("inside", Same, Same, NotSame, t.inside, D4::FLIP_X),
            q("interior", Same, Same, Same, t.interior, D4::IDENTITY),
        ]
    }

    /// Sixteen tiles chosen by which of the four sides are the same terrain. Tile `i` is the one
    /// whose side bits are `i`: up 1, right 2, down 4, left 8 — so tile 0 stands alone and tile 15
    /// is surrounded.
    pub fn edge16() -> Vec<Rule> {
        (0..16u32)
            .map(|m| {
                let side = |bit: u32| if m & bit != 0 { Cond::Same } else { Cond::NotSame };
                let mut pattern = [Cond::Any; 8];
                pattern[1] = side(1);
                pattern[4] = side(2);
                pattern[6] = side(4);
                pattern[3] = side(8);
                Rule {
                    name: format!("sides {m:04b}"),
                    pattern,
                    tile: m,
                    transform: D4::IDENTITY,
                    symmetric: false,
                }
            })
            .collect()
    }

    /// The 47 distinct neighbourhoods of the "blob" set: all eight neighbours, where a corner only
    /// counts when both sides next to it are the same terrain. Tile `i` is the `i`-th of
    /// [`blob47_masks`], in ascending order.
    pub fn blob47() -> Vec<Rule> {
        blob47_masks()
            .into_iter()
            .enumerate()
            .map(|(i, m)| {
                let mut pattern = [Cond::Any; 8];
                for (slot, bit) in BLOB_BITS.iter().enumerate() {
                    let is_corner = matches!(slot, 0 | 2 | 5 | 7);
                    if is_corner && !corner_counts(m, slot) {
                        continue;
                    }
                    pattern[slot] = if m & bit != 0 { Cond::Same } else { Cond::NotSame };
                }
                Rule {
                    name: format!("blob {m:08b}"),
                    pattern,
                    tile: i as u32,
                    transform: D4::IDENTITY,
                    symmetric: false,
                }
            })
            .collect()
    }

    /// The bit each neighbour contributes to a blob mask, in [`NEIGHBOURS`] order: up-left 128,
    /// up 1, up-right 2, left 64, right 4, down-left 32, down 16, down-right 8.
    pub const BLOB_BITS: [u8; 8] = [128, 1, 2, 64, 4, 32, 16, 8];

    /// The two sides next to a corner slot, as bits.
    fn corner_sides(slot: usize) -> u8 {
        match slot {
            0 => 1 | 64,
            2 => 1 | 4,
            5 => 16 | 64,
            7 => 16 | 4,
            _ => 0,
        }
    }

    fn corner_counts(mask: u8, slot: usize) -> bool {
        mask & corner_sides(slot) == corner_sides(slot)
    }

    /// Every blob mask with its meaningless corners cleared, ascending: 47 of them.
    pub fn blob47_masks() -> Vec<u8> {
        let mut out: Vec<u8> = (0..=255u8)
            .map(|m| {
                let mut m = m;
                for slot in [0, 2, 5, 7] {
                    if !corner_counts(m, slot) {
                        m &= !BLOB_BITS[slot];
                    }
                }
                m
            })
            .collect();
        out.sort_unstable();
        out.dedup();
        out
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
            quarters: Vec::new(),
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
    fn blob47_matches_every_neighbourhood_exactly_once() {
        let rules = blob47();
        assert_eq!(rules.len(), 47);
        for m in 0..=255u8 {
            let same = |d: IVec2| {
                let slot = NEIGHBOURS.iter().position(|n| *n == d).unwrap();
                m & BLOB_BITS[slot] != 0
            };
            let hits = rules.iter().filter(|r| r.resolve(&same).is_some()).count();
            assert_eq!(hits, 1, "mask {m:08b}");
        }
    }

    #[test]
    fn edge16_indexes_by_sides() {
        let t = Terrain {
            name: "t".into(),
            tileset: "t".into(),
            rules: edge16(),
            fallback: None,
            quarters: Vec::new(),
        };
        // Same terrain above and to the left: 1 + 8.
        let got = t.resolve(|d| d == IVec2::new(0, -1) || d == IVec2::new(-1, 0));
        assert_eq!(got, Some((9, D4::IDENTITY)));
    }

    #[test]
    fn runs_have_caps() {
        let rules = horizontal_run(0, 1, 2, 1);
        let t = Terrain {
            name: "p".into(),
            tileset: "p".into(),
            rules,
            fallback: None,
            quarters: Vec::new(),
        };
        let solid = [0, 1, 2];
        let at = |x: i32| t.resolve(|d| d.y == 0 && solid.contains(&(x + d.x))).unwrap().0;
        assert_eq!((at(0), at(1), at(2)), (0, 1, 2));
    }
}
