//! A sparse, unbounded 2D grid of bytes, stored in 64×64 chunks.
//!
//! This is the storage behind an infinite canvas: a pixel layer is a grid of palette indices and a
//! terrain layer is a grid of terrain ids. Only chunks with something in them exist. On disk each
//! chunk is deflate-compressed (less than half LZ4's size on pixel art, and pure Rust to decode on
//! the web); in human-readable dumps a grid is only summarised.

use std::collections::BTreeMap;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::geom::{IRect, IVec2};

pub const CHUNK: i32 = 64;
const CHUNK_AREA: usize = (CHUNK * CHUNK) as usize;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Grid {
    chunks: BTreeMap<(i32, i32), Box<[u8]>>,
}

impl std::fmt::Debug for Grid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Grid({} chunks)", self.chunks.len())
    }
}

fn split(p: IVec2) -> ((i32, i32), usize) {
    let key = (p.x.div_euclid(CHUNK), p.y.div_euclid(CHUNK));
    let local = (p.y.rem_euclid(CHUNK) * CHUNK + p.x.rem_euclid(CHUNK)) as usize;
    (key, local)
}

impl Grid {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, p: IVec2) -> u8 {
        let (key, local) = split(p);
        self.chunks.get(&key).map_or(0, |c| c[local])
    }

    pub fn set(&mut self, p: IVec2, value: u8) {
        let (key, local) = split(p);
        if value == 0 {
            if let Some(chunk) = self.chunks.get_mut(&key) {
                chunk[local] = 0;
            }
            return;
        }
        self.chunks
            .entry(key)
            .or_insert_with(|| vec![0; CHUNK_AREA].into_boxed_slice())[local] = value;
    }

    pub fn fill_rect(&mut self, rect: IRect, value: u8) {
        for y in rect.y..rect.max().y {
            for x in rect.x..rect.max().x {
                self.set(IVec2::new(x, y), value);
            }
        }
    }

    /// Drop chunks that hold nothing but zeros.
    pub fn prune(&mut self) {
        self.chunks.retain(|_, c| c.iter().any(|&v| v != 0));
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.values().all(|c| c.iter().all(|&v| v == 0))
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// The chunk keys, for renderers that upload chunk by chunk.
    pub fn chunk_keys(&self) -> impl Iterator<Item = IVec2> + '_ {
        self.chunks.keys().map(|&(x, y)| IVec2::new(x, y))
    }

    /// The tight bounds of every non-zero cell.
    pub fn bounds(&self) -> Option<IRect> {
        let mut out: Option<IRect> = None;
        for (&(cx, cy), chunk) in &self.chunks {
            for (i, &v) in chunk.iter().enumerate() {
                if v == 0 {
                    continue;
                }
                let p = IVec2::new(cx * CHUNK + (i as i32 % CHUNK), cy * CHUNK + (i as i32 / CHUNK));
                let cell = IRect::new(p.x, p.y, 1, 1);
                out = Some(out.map_or(cell, |r| r.union(&cell)));
            }
        }
        out
    }

    /// Every non-zero cell, in chunk order.
    pub fn iter(&self) -> impl Iterator<Item = (IVec2, u8)> + '_ {
        self.chunks.iter().flat_map(|(&(cx, cy), chunk)| {
            chunk.iter().enumerate().filter(|(_, v)| **v != 0).map(move |(i, &v)| {
                (
                    IVec2::new(cx * CHUNK + (i as i32 % CHUNK), cy * CHUNK + (i as i32 / CHUNK)),
                    v,
                )
            })
        })
    }
}

#[derive(Serialize, Deserialize)]
struct StoredChunk {
    x: i32,
    y: i32,
    deflate: Vec<u8>,
}

impl Serialize for Grid {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() {
            return s.serialize_str(&format!("<{} chunks>", self.chunks.len()));
        }
        let stored: Vec<StoredChunk> = self
            .chunks
            .iter()
            .filter(|(_, c)| c.iter().any(|&v| v != 0))
            .map(|(&(x, y), c)| StoredChunk {
                x,
                y,
                deflate: miniz_oxide::deflate::compress_to_vec(c, 9),
            })
            .collect();
        stored.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Grid {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let stored = Vec::<StoredChunk>::deserialize(d)?;
        let mut chunks = BTreeMap::new();
        for c in stored {
            let data = miniz_oxide::inflate::decompress_to_vec_with_limit(&c.deflate, CHUNK_AREA)
                .map_err(|e| D::Error::custom(format!("bad chunk: {e:?}")))?;
            if data.len() != CHUNK_AREA {
                return Err(D::Error::custom("chunk has the wrong size"));
            }
            chunks.insert((c.x, c.y), data.into_boxed_slice());
        }
        Ok(Grid { chunks })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_coordinates_are_their_own_chunks() {
        let mut g = Grid::new();
        g.set(IVec2::new(-1, -1), 3);
        g.set(IVec2::new(0, 0), 4);
        assert_eq!(g.get(IVec2::new(-1, -1)), 3);
        assert_eq!(g.get(IVec2::new(0, 0)), 4);
        assert_eq!(g.chunk_count(), 2);
        assert_eq!(g.bounds(), Some(IRect::new(-1, -1, 2, 2)));
    }

    #[test]
    fn round_trips_through_postcard() {
        let mut g = Grid::new();
        g.fill_rect(IRect::new(-70, 5, 100, 3), 9);
        let bytes = postcard::to_stdvec(&g).unwrap();
        let back: Grid = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(back, g);
    }
}
