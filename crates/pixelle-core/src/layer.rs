//! Layers of the canvas, bottom to top.

use serde::{Deserialize, Serialize};

use crate::d4::D4;
use crate::geom::IVec2;
use crate::grid::Grid;

/// Stable for the life of a project: sprites and tilesets refer to layers by id, so renaming or
/// reordering a layer never breaks an export.
pub type LayerId = u32;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    /// A guide for the artist — notes, mockups, work in progress. Drawn on the canvas, never part
    /// of an export.
    pub reference: bool,
    pub kind: LayerKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LayerKind {
    /// Palette indices, one per pixel.
    Pixels(Grid),
    /// Terrain ids painted on a coarse grid, drawn with tiles chosen by each terrain's rules.
    Terrain(TerrainLayer),
    /// Live copies of sprites: editing a sprite's pixels updates every instance of it.
    Instances(Vec<Instance>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerrainLayer {
    /// The size of a cell in pixels. Cell `(i, j)` covers pixels from `(i, j) * cell_size`.
    pub cell_size: u32,
    /// `0` is empty; `n` is `project.terrains[n - 1]`.
    pub cells: Grid,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    /// The sprite's name.
    pub sprite: String,
    pub frame: u32,
    /// Where the frame's top-left pixel lands.
    pub pos: IVec2,
    #[serde(default)]
    pub transform: D4,
    /// Palette swap slots to recolour this instance with: `(slot name, palette index)`.
    #[serde(default)]
    pub swaps: Vec<(String, u8)>,
}

impl Layer {
    pub fn pixels(&self) -> Option<&Grid> {
        match &self.kind {
            LayerKind::Pixels(grid) => Some(grid),
            _ => None,
        }
    }

    pub fn pixels_mut(&mut self) -> Option<&mut Grid> {
        match &mut self.kind {
            LayerKind::Pixels(grid) => Some(grid),
            _ => None,
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            LayerKind::Pixels(_) => "pixels",
            LayerKind::Terrain(_) => "terrain",
            LayerKind::Instances(_) => "instances",
        }
    }
}
