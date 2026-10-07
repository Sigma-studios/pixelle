//! Pixelle's core: everything about a project that is not a user interface or an engine.
//!
//! A project is a folder of text files ([`folder`]): a root holding the one palette every pixel
//! indexes into, and a file per sprite or terrain. To composite, pack and bake, a project is laid
//! out on one canvas of layers ([`Project`]), which is also what the editor's sandbox paints on.
//! Nothing here depends on egui or Bevy, so the editor, the CLI and the game loader all read the
//! same code.

pub mod aseprite;
pub mod bake;
pub mod d4;
pub mod folder;
pub mod geom;
pub mod import;
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
pub use folder::{ProjectDir, SpriteFile, TerrainFile};
pub use geom::{IRect, IVec2};
pub use grid::Grid;
pub use layer::{Instance, Layer, LayerId, LayerKind, TerrainLayer};
pub use palette::{Palette, PaletteEntry, Rgba, Section};
pub use project::{Bookmark, Project, SwapSlot};
pub use sprite::{Frame, Sprite};
pub use tiles::{Cond, Rule, Terrain, Tileset};
