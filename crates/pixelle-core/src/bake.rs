//! Baking: what a game draws from, and the `.pxlb` file a web build ships instead of the project.
//!
//! A project carries everything an artist needs — sketches, layers, terrain rules, pixels as
//! text. A game needs far less: the exported frames packed into one indexed atlas, the palette,
//! and the names, anchors, timings and recolour slots that go with them. A game reading the
//! project folder bakes it as it loads; a web build bakes it ahead of time into one compressed
//! file about the size of the PNGs it replaces.
//!
//! ```text
//! b"PXLB"  magic
//! u16 LE   format version
//! u32 LE   flags (none yet)
//! ...      postcard-encoded `Baked`; the atlas pixels inside it are deflate-compressed
//! ```
//!
//! Anchor points and rects are given in the exported frame's own coordinates — after the frame's
//! orientation is applied — so a game never has to know a frame was transposed.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::folder::ProjectDir;
use crate::geom::{IRect, IVec2};
use crate::pack;
use crate::palette::{Rgba, Section};
use crate::d4::D4;
use crate::project::{Project, SwapSlot};
use crate::sprite::{Frame, Sprite};
use crate::tiles::{Choice, QuarterRule, Rule, Terrain};

pub const MAGIC: &[u8; 4] = b"PXLB";
/// 2: terrains. 3: quarter rules for one corner. Baked files are derived, so an older one is
/// rebuilt rather than read.
pub const VERSION: u16 = 3;

#[derive(Debug)]
pub enum Error {
    NotBaked,
    UnsupportedVersion(u16),
    /// Baked by an older version: bake it again.
    Stale(u16),
    Encoding(postcard::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotBaked => write!(f, "not a baked Pixelle file"),
            Error::UnsupportedVersion(v) => write!(f, "baked with format version {v}, newer than this build reads ({VERSION})"),
            Error::Stale(v) => write!(f, "baked with format version {v}; bake it again"),
            Error::Encoding(e) => write!(f, "corrupt baked file: {e}"),
        }
    }
}

impl std::error::Error for Error {}
/// Pixels of padding between frames in the atlas, edge pixels repeated into it.
pub const GUTTER: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Baked {
    pub name: String,
    pub palette: Vec<Rgba>,
    pub palette_names: Vec<String>,
    pub sections: Vec<Section>,
    pub swaps: Vec<SwapSlot>,
    pub atlas_width: u32,
    pub atlas_height: u32,
    /// Palette indices, row-major, deflate-compressed on disk.
    #[serde(with = "deflated")]
    pub atlas: Vec<u8>,
    /// In project order.
    pub sprites: Vec<BakedSprite>,
    /// In project order: a terrain layer cell holding `n` is `terrains[n - 1]`.
    pub terrains: Vec<BakedTerrain>,
}

/// A terrain, ready to autotile a grid at run time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BakedTerrain {
    pub name: String,
    pub tile_size: u32,
    pub rules: Vec<Rule>,
    pub quarters: Vec<QuarterRule>,
    pub fallback: Option<u32>,
    /// Each tile of the tileset in the atlas, as drawn and transposed.
    pub tiles: Vec<BakedTile>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BakedTile {
    pub plain: IRect,
    pub transposed: IRect,
}

/// One sprite to draw for a terrain cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    /// Where to sample, in atlas pixels.
    pub source: IRect,
    /// Where it goes within the cell, in pixels, `y` down.
    pub at: IRect,
    pub flip_x: bool,
    pub flip_y: bool,
}

impl BakedTerrain {
    fn terrain(&self) -> Terrain {
        Terrain {
            name: self.name.clone(),
            tileset: String::new(),
            rules: self.rules.clone(),
            fallback: self.fallback,
            quarters: self.quarters.clone(),
        }
    }

    /// The sprites that draw a cell, given which neighbours (offsets, `y` down) are the same
    /// terrain. Empty if nothing matches.
    pub fn cell(&self, same: impl Fn(IVec2) -> bool) -> Vec<Piece> {
        let frame = |tile: u32, t: D4| {
            let b = self.tiles.get(tile as usize).copied();
            b.map(|b| if t.transpose { b.transposed } else { b.plain })
        };
        let s = self.tile_size;
        match self.terrain().choose(same) {
            None => Vec::new(),
            Some(Choice::Whole { tile, transform }) => frame(tile, transform)
                .map(|source| Piece { source, at: IRect::new(0, 0, s, s), flip_x: transform.flip_x, flip_y: transform.flip_y })
                .into_iter()
                .collect(),
            Some(Choice::Quarters(quarters)) => {
                let h = s / 2;
                quarters
                    .iter()
                    .filter_map(|q| {
                        let base = frame(q.tile, q.transform)?;
                        // The top-left quarter of the flipped frame is the opposite quarter of
                        // the frame itself, flipped.
                        let sx = if q.transform.flip_x { h as i32 } else { 0 };
                        let sy = if q.transform.flip_y { h as i32 } else { 0 };
                        let ax = if q.place.flip_x { h as i32 } else { 0 };
                        let ay = if q.place.flip_y { h as i32 } else { 0 };
                        Some(Piece {
                            source: IRect::new(base.x + sx, base.y + sy, h, h),
                            at: IRect::new(ax, ay, h, h),
                            flip_x: q.transform.flip_x != q.place.flip_x,
                            flip_y: q.transform.flip_y != q.place.flip_y,
                        })
                    })
                    .collect()
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BakedSprite {
    pub name: String,
    pub frames: Vec<BakedFrame>,
    /// In exported-frame pixels; `None` is the centre.
    pub pivot: Option<IVec2>,
    pub tags: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BakedFrame {
    /// Where the frame is in the atlas.
    pub rect: IRect,
    pub duration: f32,
    pub points: BTreeMap<String, IVec2>,
    pub rects: BTreeMap<String, IRect>,
}

impl BakedSprite {
    pub fn is_animation(&self) -> bool {
        self.frames.len() > 1 && self.frames.iter().any(|f| f.duration > 0.0)
    }
}

impl Baked {
    pub fn terrain(&self, name: &str) -> Option<&BakedTerrain> {
        self.terrains.iter().find(|t| t.name == name)
    }

    pub fn sprite(&self, name: &str) -> Option<&BakedSprite> {
        self.sprites.iter().find(|s| s.name == name)
    }

    /// The atlas as straight RGBA, through the palette.
    pub fn atlas_rgba(&self) -> Vec<u8> {
        self.rgba_through(&self.palette)
    }

    /// The atlas as straight RGBA through a recoloured palette: `swaps` are `(slot name, index)`.
    pub fn atlas_rgba_swapped(&self, swaps: &[(&str, u8)]) -> Vec<u8> {
        let mut palette = self.palette.clone();
        for (slot, to) in swaps {
            if let Some(s) = self.swaps.iter().find(|s| s.name == *slot)
                && let Some(c) = self.palette.get(*to as usize)
            {
                palette[s.index as usize] = *c;
            }
        }
        self.rgba_through(&palette)
    }

    fn rgba_through(&self, palette: &[Rgba]) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.atlas.len() * 4);
        for &v in &self.atlas {
            out.extend_from_slice(&palette.get(v as usize).copied().unwrap_or([0; 4]));
        }
        out
    }

    /// The palette indices a swap slot may take: its options section.
    pub fn swap_options(&self, slot: &str) -> Option<std::ops::Range<u16>> {
        let s = self.swaps.iter().find(|s| s.name == slot)?;
        let sec = self.sections.iter().find(|x| x.label == s.options)?;
        Some(sec.start..sec.start + sec.len)
    }

    /// A sprite's frames side by side, top-aligned, as straight RGBA: the sheet an artist would
    /// have exported by hand.
    pub fn sheet_rgba(&self, sprite: &str) -> Option<crate::render::RgbaImage> {
        let s = self.sprite(sprite)?;
        let w = s.frames.iter().map(|f| f.rect.w).sum();
        let h = s.frames.iter().map(|f| f.rect.h).max().unwrap_or(0);
        let mut out = crate::render::RgbaImage::new(w, h);
        let mut x0 = 0;
        for f in &s.frames {
            for y in 0..f.rect.h {
                for x in 0..f.rect.w {
                    let v = self.atlas[((f.rect.y as u32 + y) * self.atlas_width + f.rect.x as u32 + x) as usize];
                    let c = self.palette.get(v as usize).copied().unwrap_or([0; 4]);
                    let at = (((y * w) + x0 + x) * 4) as usize;
                    out.px[at..at + 4].copy_from_slice(&c);
                }
            }
            x0 += f.rect.w;
        }
        Some(out)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&postcard::to_stdvec(self).expect("a baked file always encodes"));
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Baked, Error> {
        if bytes.len() < 10 || &bytes[..4] != MAGIC {
            return Err(Error::NotBaked);
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version > VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        if version < VERSION {
            return Err(Error::Stale(version));
        }
        postcard::from_bytes(&bytes[10..]).map_err(Error::Encoding)
    }
}

/// Bake a project.
pub fn bake(project: &Project) -> Baked {
    // Terrain tiles go in the atlas as sprites of their own, each as drawn and transposed.
    let mut with_tiles = project.clone();
    let tile_name = |t: &str, i: u32, transposed: bool| format!("\0terrain/{t}/{i}/{}", u8::from(transposed));
    for terrain in &project.terrains {
        let Some(ts) = project.tileset(&terrain.tileset) else { continue };
        for i in 0..ts.count {
            for transposed in [false, true] {
                let mut f = Frame::new(ts.tile_rect(i));
                if transposed {
                    f.transform = D4::TRANSPOSE;
                }
                with_tiles.sprites.push(Sprite::new(tile_name(&terrain.name, i, transposed), ts.layers.clone(), vec![f]));
            }
        }
    }
    let atlas = pack::pack(&with_tiles, GUTTER);
    let terrains = project
        .terrains
        .iter()
        .map(|t| {
            let ts = project.tileset(&t.tileset);
            let count = ts.map_or(0, |ts| ts.count);
            BakedTerrain {
                name: t.name.clone(),
                tile_size: ts.map_or(0, |ts| ts.tile_size),
                rules: t.rules.clone(),
                quarters: t.quarters.clone(),
                fallback: t.fallback,
                tiles: (0..count)
                    .map(|i| BakedTile {
                        plain: atlas.sprites[&tile_name(&t.name, i, false)][0],
                        transposed: atlas.sprites[&tile_name(&t.name, i, true)][0],
                    })
                    .collect(),
            }
        })
        .collect();
    let sprites = project
        .sprites
        .iter()
        .map(|s| {
            let rects = &atlas.sprites[&s.name];
            BakedSprite {
                name: s.name.clone(),
                pivot: s.pivot.map(|p| {
                    let f = &s.frames[0];
                    f.transform.apply(p, f.rect.w, f.rect.h)
                }),
                tags: s.tags.clone(),
                frames: s
                    .frames
                    .iter()
                    .zip(rects)
                    .map(|(f, r)| {
                        let (w, h) = (f.rect.w, f.rect.h);
                        BakedFrame {
                            rect: *r,
                            duration: f.duration,
                            points: f
                                .points
                                .iter()
                                .map(|(k, p)| (k.clone(), f.transform.apply(*p, w, h)))
                                .collect(),
                            rects: f
                                .rects
                                .iter()
                                .map(|(k, rect)| {
                                    let a = f.transform.apply(rect.min(), w, h);
                                    let b = f.transform.apply(rect.max() - IVec2::ONE, w, h);
                                    (k.clone(), IRect::from_min_max(a.min(b), a.max(b) + IVec2::ONE))
                                })
                                .collect(),
                        }
                    })
                    .collect(),
            }
        })
        .collect();
    Baked {
        name: project.name.clone(),
        palette: project.palette.entries().iter().map(|e| e.rgba).collect(),
        palette_names: project.palette.entries().iter().map(|e| e.name.clone()).collect(),
        sections: project.palette.sections().to_vec(),
        swaps: project.swaps.clone(),
        atlas_width: atlas.width,
        atlas_height: atlas.height,
        atlas: atlas.pixels,
        sprites,
        terrains,
    }
}

/// Bake a project folder.
pub fn bake_folder(project: &ProjectDir) -> Baked {
    bake(&project.to_canvas())
}

mod deflated {
    use serde::de::Error as _;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(&miniz_oxide::deflate::compress_to_vec(v, 9))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let packed: Vec<u8> = serde_bytes_vec(d)?;
        miniz_oxide::inflate::decompress_to_vec_with_limit(&packed, 64 << 20)
            .map_err(|e| D::Error::custom(format!("bad atlas: {e:?}")))
    }

    fn serde_bytes_vec<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Vec<u8>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "bytes")
            }
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Vec<u8>, E> {
                Ok(v.to_vec())
            }
            fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Vec<u8>, E> {
                Ok(v)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<u8>, A::Error> {
                let mut out = Vec::new();
                while let Some(b) = seq.next_element()? {
                    out.push(b);
                }
                Ok(out)
            }
        }
        d.deserialize_bytes(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Grid;
    use crate::layer::LayerKind;
    use crate::sprite::{Frame, Sprite};

    #[test]
    fn bakes_round_trips_and_orients_anchors() {
        let mut p = Project::new("t");
        p.palette.begin_section("S");
        let a = p.palette.push("a", [9, 9, 9, 255]).unwrap();
        let mut g = Grid::new();
        g.fill_rect(IRect::new(0, 0, 4, 2), a);
        let art = p.add_layer("art", LayerKind::Pixels(g));
        p.sprites.push(Sprite::new(
            "s",
            vec![art],
            vec![
                Frame::new(IRect::new(0, 0, 4, 2))
                    .with_transform(D4::TRANSPOSE)
                    .with_point("tip", IVec2::new(3, 0))
                    .with_rect("r", IRect::new(1, 0, 2, 1)),
            ],
        ));
        let baked = bake(&p);
        let f = &baked.sprite("s").unwrap().frames[0];
        assert_eq!((f.rect.w, f.rect.h), (2, 4));
        assert_eq!(f.points["tip"], IVec2::new(0, 3));
        assert_eq!(f.rects["r"], IRect::new(0, 1, 1, 2));
        let back = Baked::from_bytes(&baked.to_bytes()).unwrap();
        assert_eq!(back, baked);
    }
}

#[cfg(test)]
mod terrain_tests {
    use super::*;
    use crate::grid::Grid;
    use crate::layer::{LayerKind, TerrainLayer};
    use crate::render::{Indexed, Renderer};
    use crate::tiles::{Tileset, presets};

    /// Every neighbourhood, drawn from baked pieces, matches the editor's own rendering.
    #[test]
    fn baked_pieces_draw_what_the_editor_draws() {
        let mut p = Project::new("t");
        p.palette.begin_section("S");
        for i in 0..40 {
            p.palette.push(format!("{i}"), [i * 6, 255 - i * 6, i, 255]).unwrap();
        }
        // Seven 8x8 tiles, every pixel different, so any wrong flip or quarter shows.
        let mut g = Grid::new();
        for t in 0..7 {
            for y in 0..8 {
                for x in 0..8 {
                    g.set(IVec2::new(t * 8 + x, y), 1 + ((t * 5 + x * 3 + y * 7) % 39) as u8);
                }
            }
        }
        let art = p.add_layer("art", LayerKind::Pixels(g));
        p.tilesets.push(Tileset { name: "w".into(), origin: IVec2::ZERO, tile_size: 8, columns: 7, count: 7, layers: vec![art], gap: 0 });
        let roles = presets::MinimalD4 { corner: 0, edge: 1, strip_end: 2, strip: 3, interior: 4, inside: 5, double_inside: 6 };
        let mut t = Terrain::new("walls", "w", presets::minimal_d4(roles));
        t.quarters = presets::quarters_d4(roles);
        p.terrains.push(t);
        let level = p.add_layer("level", LayerKind::Terrain(TerrainLayer { cell_size: 8, cells: Grid::new() }));

        let baked = bake(&p);
        let atlas = Indexed { w: baked.atlas_width, h: baked.atlas_height, px: baked.atlas.clone() };
        let bt = baked.terrain("walls").unwrap();
        let origin = IVec2::new(50, 50);
        for mask in 0..=255u8 {
            let same = |d: IVec2| {
                crate::tiles::NEIGHBOURS.iter().position(|n| *n == d).is_none_or(|s| mask & (1 << s) != 0)
            };
            if let LayerKind::Terrain(tl) = &mut p.layer_mut(level).unwrap().kind {
                tl.cells = Grid::new();
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        if same(IVec2::new(dx, dy)) {
                            tl.cells.set(origin + IVec2::new(dx, dy), 1);
                        }
                    }
                }
            }
            let want = Renderer::new(&p).indexed(IRect::new(400, 400, 8, 8), &[level]);
            let mut got = Indexed::new(8, 8);
            for piece in bt.cell(same) {
                let src = atlas.crop(piece.source.x as u32, piece.source.y as u32, piece.source.w, piece.source.h);
                let src = src.transformed(D4::new(false, piece.flip_x, piece.flip_y));
                got.blit(&src, piece.at.min());
            }
            assert_eq!(got, want, "neighbourhood {mask:08b}");
        }
    }
}
