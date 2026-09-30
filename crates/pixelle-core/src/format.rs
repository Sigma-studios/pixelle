//! The `.pxl` file: one binary file per project.
//!
//! ```text
//! b"PXLL"  magic
//! u16 LE   format version
//! u32 LE   flags (none yet)
//! ...      the project, postcard-encoded; pixel chunks inside it are deflate-compressed
//! ```

use crate::project::Project;

pub const MAGIC: &[u8; 4] = b"PXLL";
/// 2: terrains gained quarter rules. Version 1 files still load.
pub const VERSION: u16 = 2;
const HEADER: usize = 10;

#[derive(Debug)]
pub enum Error {
    NotPixelle,
    UnsupportedVersion(u16),
    /// A derived file from an older version: build it again.
    Stale(u16),
    Encoding(postcard::Error),
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotPixelle => write!(f, "not a Pixelle file"),
            Error::UnsupportedVersion(v) => {
                write!(f, "file format version {v} is newer than this build reads ({VERSION})")
            }
            Error::Stale(v) => write!(f, "baked with format version {v}; bake it again"),
            Error::Encoding(e) => write!(f, "corrupt project: {e}"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub fn save(project: &Project) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(4096);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    let body = postcard::to_stdvec(project).map_err(Error::Encoding)?;
    out.extend_from_slice(&body);
    Ok(out)
}

pub fn load(bytes: &[u8]) -> Result<Project, Error> {
    if bytes.len() < HEADER || &bytes[..4] != MAGIC {
        return Err(Error::NotPixelle);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version > VERSION {
        return Err(Error::UnsupportedVersion(version));
    }
    crate::tiles::compat::FORMAT.with(|f| f.set(version));
    let project = postcard::from_bytes(&bytes[HEADER..]).map_err(Error::Encoding);
    crate::tiles::compat::FORMAT.with(|f| f.set(VERSION));
    project
}

pub fn save_file(project: &Project, path: impl AsRef<std::path::Path>) -> Result<(), Error> {
    // Write beside and rename, so a crash mid-save never leaves half a project.
    let path = path.as_ref();
    let tmp = path.with_extension("pxl.tmp");
    std::fs::write(&tmp, save(project)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

pub fn load_file(path: impl AsRef<std::path::Path>) -> Result<Project, Error> {
    load(&std::fs::read(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{IRect, IVec2};
    use crate::grid::Grid;
    use crate::layer::LayerKind;
    use crate::sprite::{Frame, Sprite};

    #[test]
    fn round_trips() {
        let mut p = Project::new("t");
        p.palette.begin_section("S");
        p.palette.push("a", [1, 2, 3, 255]).unwrap();
        let mut g = Grid::new();
        g.fill_rect(IRect::new(-100, -3, 300, 7), 1);
        let art = p.add_layer("art", LayerKind::Pixels(g));
        p.sprites.push(Sprite::new(
            "s",
            vec![art],
            vec![Frame::new(IRect::new(0, 0, 8, 8)).with_point("muzzle", IVec2::new(6, 3))],
        ));
        let bytes = save(&p).unwrap();
        assert_eq!(&bytes[..4], MAGIC);
        assert_eq!(load(&bytes).unwrap(), p);
        assert!(matches!(load(b"nope nope nope"), Err(Error::NotPixelle)));
    }

    #[test]
    fn version_1_files_still_load() {
        let mut p = Project::new("t");
        p.terrains.push(crate::tiles::Terrain::new("t", "ts", Vec::new()));
        // A version 1 file is the same bytes without the terrain's quarters: write one by hand.
        #[derive(serde::Serialize)]
        struct V1Terrain<'a> {
            name: &'a str,
            tileset: &'a str,
            rules: Vec<crate::tiles::Rule>,
            fallback: Option<u32>,
        }
        let mut bytes = save(&p).unwrap();
        let v2_terrain = postcard::to_stdvec(&p.terrains[0]).unwrap();
        let v1_terrain = postcard::to_stdvec(&V1Terrain { name: "t", tileset: "ts", rules: Vec::new(), fallback: None }).unwrap();
        let at = bytes.windows(v2_terrain.len()).position(|w| w == v2_terrain).unwrap();
        bytes.splice(at..at + v2_terrain.len(), v1_terrain);
        bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(load(&bytes).unwrap(), p);
    }
}
