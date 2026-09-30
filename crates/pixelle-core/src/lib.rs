//! Pixelle's core: everything about a project that is not a user interface or an engine.
//!
//! A project is one infinite canvas of layers holding palette indices, plus the metadata that
//! turns regions of it into game assets: named sprites (static or animated, with anchor points),
//! tilesets that are themselves regions of the canvas, and terrains whose rules pick tiles for a
//! painted grid. Nothing here depends on egui or Bevy, so the editor, the CLI and the game loader
//! all read the same code.

pub mod d4;
pub mod format;
pub mod geom;
pub mod grid;
pub mod layer;
pub mod pack;
pub mod palette;
#[cfg(feature = "png")]
pub mod png_io;
pub mod project;
pub mod render;
pub mod sprite;
pub mod tiles;

pub use d4::D4;
pub use geom::{IRect, IVec2};
pub use grid::Grid;
pub use layer::{Instance, Layer, LayerId, LayerKind, TerrainLayer};
pub use palette::{Palette, PaletteEntry, Rgba, Section};
pub use project::{Bookmark, Project, SwapSlot};
pub use sprite::{Frame, Sprite};
pub use tiles::{Cond, Rule, Terrain, Tileset};
