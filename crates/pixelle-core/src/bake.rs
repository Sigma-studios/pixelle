//! The baked runtime file (`.pxlb`): what a game ships instead of the project.
//!
//! A project carries everything an artist needs — mockups, sketches, the whole canvas, terrain
//! rules. A game needs far less: the exported frames packed into one indexed atlas, the palette,
//! and the names, anchors, timings and recolour slots that go with them. Baking keeps exactly
//! that, so a release build downloads a file about the size of the PNGs it replaces, and loading
//! it is a decompress and a palette lookup rather than a composite of the canvas.
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

use crate::format;
use crate::geom::{IRect, IVec2};
use crate::pack;
use crate::palette::{Rgba, Section};
use crate::project::{Project, SwapSlot};

pub const MAGIC: &[u8; 4] = b"PXLB";
pub const VERSION: u16 = 1;
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

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&postcard::to_stdvec(self).expect("a baked file always encodes"));
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Baked, format::Error> {
        if bytes.len() < 10 || &bytes[..4] != MAGIC {
            return Err(format::Error::NotPixelle);
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version > VERSION {
            return Err(format::Error::UnsupportedVersion(version));
        }
        postcard::from_bytes(&bytes[10..]).map_err(format::Error::Encoding)
    }
}

/// Bake a project.
pub fn bake(project: &Project) -> Baked {
    let atlas = pack::pack(project, GUTTER);
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
    }
}

/// Read either kind of file: a project is baked on the spot, a baked file is decoded.
pub fn load_any(bytes: &[u8]) -> Result<Baked, format::Error> {
    if bytes.starts_with(MAGIC) {
        Baked::from_bytes(bytes)
    } else {
        Ok(bake(&format::load(bytes)?))
    }
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
    use crate::d4::D4;
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
        assert_eq!(load_any(&format::save(&p).unwrap()).unwrap(), baked);
    }
}
