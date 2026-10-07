//! The open project folder, and how each of its files becomes a canvas to work on.
//!
//! The project lives here as files ([`ProjectDir`]). A file being edited is laid out on a canvas
//! of its own — a sprite's frames in a strip, a terrain's tiles in labelled rows over a patch of
//! ground to try them on — so that every tool works on it the way it works anywhere; when the
//! file is saved, or anything else needs it, the canvas is read back into the file. The Sandbox
//! is a canvas that is never read back: somewhere to try the project's terrains.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use pixelle_core::folder::{self, ProjectDir, SpriteFile, SpriteFrame, SpriteLayer, TerrainFile};
use pixelle_core::layer::{LayerKind, TerrainLayer};
use pixelle_core::render::Indexed;
use pixelle_core::tiles::Tileset;
use pixelle_core::{Frame, Grid, IRect, IVec2, Project, Sprite};

/// What a tab shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabKind {
    Sandbox,
    Sprite(String),
    Terrain(String),
}

impl TabKind {
    pub fn asset(&self) -> Option<&str> {
        match self {
            TabKind::Sandbox => None,
            TabKind::Sprite(n) | TabKind::Terrain(n) => Some(n),
        }
    }

    pub fn label(&self) -> String {
        match self {
            TabKind::Sandbox => "Sandbox".into(),
            TabKind::Sprite(n) | TabKind::Terrain(n) => n.rsplit('/').next().unwrap_or(n).to_string(),
        }
    }
}

/// Pixels between frames in a sprite's strip, and between tiles in a terrain's sheet.
pub fn gap(size: u32) -> u32 {
    (size / 2).clamp(2, 8)
}

/// A canvas with the project's palette and recolour slots.
fn blank(project: &ProjectDir) -> Project {
    let mut p = Project::new(project.name.clone());
    p.grid = project.grid;
    p.palette = project.palette.clone();
    p.swaps = project.swaps.clone();
    p
}

fn paint(grid: &mut Grid, pic: &Indexed, at: IVec2) {
    for y in 0..pic.h {
        for x in 0..pic.w {
            let v = pic.get(x, y);
            if v != 0 {
                grid.set(at + IVec2::new(x as i32, y as i32), v);
            }
        }
    }
}

fn cut(grid: &Grid, r: IRect) -> Indexed {
    let mut out = Indexed::new(r.w, r.h);
    for y in 0..r.h {
        for x in 0..r.w {
            out.set(x, y, grid.get(r.min() + IVec2::new(x as i32, y as i32)));
        }
    }
    out
}

/// The tag that carries frame names across a sprite canvas.
const FRAME_NAMES: &str = "frames";

/// A sprite's frames in a strip, left to right, each layer of it a layer of the canvas.
pub fn sprite_canvas(project: &ProjectDir, name: &str, s: &SpriteFile) -> Project {
    let mut p = blank(project);
    let ids: Vec<_> = s
        .layers
        .iter()
        .map(|l| {
            let id = p.add_layer(l.name.clone(), LayerKind::Pixels(Grid::new()));
            let layer = p.layer_mut(id).expect("just added");
            layer.visible = l.visible;
            layer.reference = !l.export;
            id
        })
        .collect();
    let gap = gap(s.frames.iter().map(|f| f.w.min(f.h)).max().unwrap_or(8)) as i32;
    let mut x = 0;
    let mut frames = Vec::new();
    for f in &s.frames {
        for (l, id) in s.layers.iter().zip(&ids) {
            if let Some(pic) = f.pixels.get(&l.name)
                && let Some(LayerKind::Pixels(grid)) = p.layer_mut(*id).map(|l| &mut l.kind)
            {
                paint(grid, pic, IVec2::new(x, 0));
            }
        }
        frames.push(Frame {
            rect: IRect::new(x, 0, f.w, f.h),
            transform: f.transform,
            duration: f.duration,
            points: f.points.clone(),
            rects: f.rects.clone(),
        });
        x += f.w as i32 + gap;
    }
    let exported = s.layers.iter().zip(&ids).filter(|(l, _)| l.export).map(|(_, id)| *id).collect();
    let mut sprite = Sprite::new(name, exported, frames);
    sprite.pivot = s.pivot;
    sprite.tags = s.tags.clone();
    if s.frames.iter().any(|f| f.name.is_some()) {
        let names: Vec<String> = s.frames.iter().map(|f| f.name.clone().unwrap_or_default()).collect();
        sprite.tags.insert(FRAME_NAMES.into(), names.join(", "));
    }
    p.sprites.push(sprite);
    p
}

/// A sprite canvas read back into its file.
pub fn canvas_sprite(p: &Project) -> SpriteFile {
    let mut s = SpriteFile::new(1, 1);
    s.frames.clear();
    let pixel_layers: Vec<_> = p.layers.iter().filter(|l| l.pixels().is_some()).collect();
    s.layers = pixel_layers.iter().map(|l| SpriteLayer { name: l.name.clone(), export: !l.reference, visible: l.visible }).collect();
    let Some(sprite) = p.sprites.first() else { return s };
    let mut tags = sprite.tags.clone();
    let names: Vec<Option<String>> = tags
        .remove(FRAME_NAMES)
        .map(|n| n.split(',').map(|n| Some(n.trim().to_string()).filter(|n| !n.is_empty())).collect())
        .unwrap_or_default();
    s.tags = tags;
    s.pivot = sprite.pivot;
    for (i, f) in sprite.frames.iter().enumerate() {
        let mut frame = SpriteFrame::new(f.rect.w, f.rect.h);
        frame.name = names.get(i).cloned().flatten();
        frame.duration = f.duration;
        frame.transform = f.transform;
        frame.points = f.points.clone();
        frame.rects = f.rects.clone();
        for l in &pixel_layers {
            frame.set_layer(&l.name, cut(l.pixels().expect("a pixel layer"), f.rect));
        }
        s.frames.push(frame);
    }
    s
}

/// A sprite canvas laid out again after `f` changed its file: frames added, removed, reordered
/// or resized.
pub fn reshape_sprite(p: &Project, f: impl FnOnce(&mut SpriteFile)) -> Project {
    let mut s = canvas_sprite(p);
    f(&mut s);
    let name = p.sprites.first().map_or(String::new(), |s| s.name.clone());
    let mut dir = ProjectDir::new(p.name.clone());
    dir.grid = p.grid;
    dir.palette = p.palette.clone();
    dir.swaps = p.swaps.clone();
    sprite_canvas(&dir, &name, &s)
}

/// Make a frame `w`×`h`, keeping its pixels where they are from the top-left.
pub fn resize_frame(f: &mut SpriteFrame, w: u32, h: u32) {
    let layers: Vec<String> = f.pixels.keys().cloned().collect();
    for l in layers {
        let mut pic = Indexed::new(w, h);
        pic.blit(&f.pixels[&l], IVec2::ZERO);
        f.set_layer(&l, pic);
    }
    f.w = w;
    f.h = h;
}

/// Columns of a terrain's tile sheet: four, so the corners, edges and ends of the preset each
/// make a row.
const TILE_COLUMNS: u32 = 4;

/// How many cells the patch of ground under a terrain's tiles is.
pub const PATCH: IVec2 = IVec2::new(12, 8);

/// A terrain's tiles in labelled rows, over a patch of ground drawn with them.
pub fn terrain_canvas(project: &ProjectDir, name: &str, t: &TerrainFile) -> Project {
    let mut p = blank(project);
    let gap = gap(t.cell);
    let tiles = p.add_layer("tiles", LayerKind::Pixels(Grid::new()));
    let ts = Tileset { name: name.into(), origin: IVec2::ZERO, tile_size: t.cell, columns: TILE_COLUMNS, count: t.tiles.len() as u32, layers: vec![tiles], gap };
    if let Some(LayerKind::Pixels(grid)) = p.layer_mut(tiles).map(|l| &mut l.kind) {
        for (i, pic) in t.tiles.iter().enumerate() {
            paint(grid, pic, ts.tile_rect(i as u32).min());
        }
    }
    // The patch starts a couple of cells below the sheet, on the cell grid.
    let below = ts.bounds().max().y.div_euclid(t.cell as i32) + 2;
    let mut cells = Grid::new();
    for (y, row) in SAMPLE.iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            if c == '#' {
                cells.set(IVec2::new(x as i32, below + y as i32), 1);
            }
        }
    }
    let ground = p.add_layer("ground", LayerKind::Terrain(TerrainLayer { cell_size: t.cell, cells }));
    p.layer_mut(ground).expect("just added").reference = true;
    p.tilesets.push(ts);
    p.terrains.push(t.terrain(name));
    p
}

/// The ground a terrain tab starts with: something of every kind of edge and corner.
const SAMPLE: [&str; 8] = [
    "............",
    ".####...#...",
    ".####...#...",
    ".##.#...###.",
    ".####.......",
    "......#####.",
    "..###.#####.",
    "......#.#.#.",
];

/// The patch of ground of a terrain canvas, in cells.
pub fn patch_rect(p: &Project) -> Option<IRect> {
    let ts = p.tilesets.first()?;
    let below = ts.bounds().max().y.div_euclid(ts.tile_size as i32) + 2;
    Some(IRect::new(0, below, PATCH.x as u32, PATCH.y as u32))
}

/// A terrain canvas read back into its file.
pub fn canvas_terrain(p: &Project, old: &TerrainFile) -> TerrainFile {
    let mut t = old.clone();
    let (Some(ts), Some(grid)) = (p.tilesets.first(), p.layers.iter().find_map(|l| l.pixels())) else { return t };
    t.tiles = (0..ts.count).map(|i| cut(grid, ts.tile_rect(i))).collect();
    t
}

/// The Sandbox's tiles: every terrain of the project, as tilesets far off on a hidden layer.
const SANDBOX_TILES: IVec2 = IVec2::new(0, -1_000_000);

/// A fresh Sandbox: a patch of ground to paint any terrain on, and pixels to doodle with.
pub fn sandbox_canvas(project: &ProjectDir) -> Project {
    let mut p = blank(project);
    p.add_layer("tiles", LayerKind::Pixels(Grid::new()));
    p.add_layer("ground", LayerKind::Terrain(TerrainLayer { cell_size: project.grid.max(1), cells: Grid::new() }));
    p.add_layer("pixels", LayerKind::Pixels(Grid::new()));
    p.layer_mut(1).expect("the tiles layer").visible = false;
    p.layer_mut(1).expect("the tiles layer").locked = true;
    refresh_sandbox(&mut p, project);
    p
}

/// Bring the Sandbox's terrains up to date with the project, keeping what is painted.
pub fn refresh_sandbox(p: &mut Project, project: &ProjectDir) {
    p.palette = project.palette.clone();
    p.swaps = project.swaps.clone();
    let Some(tiles) = p.layers.iter().find(|l| l.name == "tiles" && !l.visible).map(|l| l.id) else { return };
    let mut grid = Grid::new();
    p.tilesets.clear();
    p.terrains.clear();
    let mut y = SANDBOX_TILES.y;
    for (name, t) in &project.terrains {
        let ts = Tileset {
            name: name.clone(),
            origin: IVec2::new(SANDBOX_TILES.x, y),
            tile_size: t.cell,
            columns: t.tiles.len().max(1) as u32,
            count: t.tiles.len() as u32,
            layers: vec![tiles],
            gap: 0,
        };
        for (i, pic) in t.tiles.iter().enumerate() {
            paint(&mut grid, pic, ts.tile_rect(i as u32).min());
        }
        y += t.cell as i32 * 2;
        p.tilesets.push(ts);
        p.terrains.push(t.terrain(name));
    }
    if let Some(LayerKind::Pixels(g)) = p.layer_mut(tiles).map(|l| &mut l.kind) {
        *g = grid;
    }
    // Ground painted with a terrain that is gone is left as it is: it draws nothing.
    if let Some(ground) = p.layers.iter_mut().find_map(|l| match &mut l.kind {
        LayerKind::Terrain(t) => Some(t),
        _ => None,
    }) && let Some(first) = project.terrains.values().next()
    {
        ground.cell_size = first.cell;
    }
}

/// What the canvas of `kind` holds, read back into the project.
pub fn write_back(project: &mut ProjectDir, kind: &TabKind, canvas: &Project) {
    match kind {
        TabKind::Sandbox => {}
        TabKind::Sprite(name) => {
            let s = canvas_sprite(canvas);
            project.sprites.insert(name.clone(), s);
        }
        TabKind::Terrain(name) => {
            if let Some(old) = project.terrains.get(name) {
                let t = canvas_terrain(canvas, old);
                project.terrains.insert(name.clone(), t);
            }
        }
    }
}

/// The canvas for a file of the project.
pub fn canvas_for(project: &ProjectDir, kind: &TabKind) -> Option<Project> {
    Some(match kind {
        TabKind::Sandbox => sandbox_canvas(project),
        TabKind::Sprite(n) => sprite_canvas(project, n, project.sprites.get(n)?),
        TabKind::Terrain(n) => terrain_canvas(project, n, project.terrains.get(n)?),
    })
}

/// The project folder on disk, as far as the editor knows it.
pub struct Disk {
    pub dir: PathBuf,
    /// Each file's modification time when last read or written, by path relative to the folder.
    pub seen: BTreeMap<String, Option<SystemTime>>,
}

pub fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl Disk {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, seen: BTreeMap::new() }
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }

    /// Remember every file as it is now.
    pub fn note_all(&mut self, project: &ProjectDir) {
        self.seen.clear();
        for rel in project.to_files().keys() {
            self.note(rel);
        }
    }

    pub fn note(&mut self, rel: &str) {
        let t = mtime(&self.path(rel));
        self.seen.insert(rel.to_string(), t);
    }

    /// Files whose modification time is not what we last saw: changed, gone, or new.
    pub fn changed(&self) -> Vec<String> {
        let mut out: Vec<String> = self.seen.iter().filter(|(rel, t)| mtime(&self.path(rel)) != **t).map(|(r, _)| r.clone()).collect();
        for rel in self.scan() {
            if !self.seen.contains_key(&rel) {
                out.push(rel);
            }
        }
        out
    }

    /// Every `.pxl` file in the folder, relative, `/`-separated; hidden folders are skipped.
    pub fn scan(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut dirs = vec![(self.dir.clone(), String::new())];
        while let Some((dir, prefix)) = dirs.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                let path = e.path();
                if path.is_dir() {
                    if prefix.matches('/').count() < 8 {
                        dirs.push((path, format!("{prefix}{name}/")));
                    }
                } else if name.ends_with(".pxl") {
                    out.insert(format!("{prefix}{name}"));
                }
            }
        }
        out
    }

    /// Where unsaved work is kept, for when the editor does not get to save it.
    pub fn autosave_dir(&self) -> PathBuf {
        self.dir.join(".pixelle").join("autosave")
    }

    /// Write `files` (relative path → text) as the autosave, replacing any older one.
    pub fn autosave(&self, files: &BTreeMap<String, String>) -> std::io::Result<()> {
        let dir = self.autosave_dir();
        let _ = std::fs::remove_dir_all(&dir);
        if files.is_empty() {
            return Ok(());
        }
        let hidden = self.dir.join(".pixelle");
        std::fs::create_dir_all(&hidden)?;
        let ignore = hidden.join(".gitignore");
        if !ignore.exists() {
            std::fs::write(ignore, "*\n")?;
        }
        for (rel, text) in files {
            folder::write_atomic(&dir.join(rel), text.as_bytes())?;
        }
        Ok(())
    }

    /// The autosaved files, if any are newer than what they stand in for.
    pub fn recoverable(&self) -> BTreeMap<String, String> {
        let dir = self.autosave_dir();
        let mut out = BTreeMap::new();
        let mut dirs = vec![(dir.clone(), String::new())];
        while let Some((d, prefix)) = dirs.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else { continue };
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let path = e.path();
                if path.is_dir() {
                    dirs.push((path, format!("{prefix}{name}/")));
                } else if name.ends_with(".pxl") {
                    let rel = format!("{prefix}{name}");
                    let newer = match (mtime(&path), mtime(&self.path(&rel))) {
                        (Some(a), Some(b)) => a > b,
                        _ => true,
                    };
                    if newer && let Ok(text) = std::fs::read_to_string(&path) {
                        out.insert(rel, text);
                    }
                }
            }
        }
        out
    }

    pub fn clear_autosave(&self) {
        let _ = std::fs::remove_dir_all(self.autosave_dir());
    }
}

/// A name for a new asset that nothing has yet: `base`, `base 2`, …
pub fn free_name(project: &ProjectDir, base: &str) -> String {
    let base = base.trim().trim_matches('/');
    let base = if base.is_empty() { "untitled" } else { base };
    if project.kind(base).is_none() {
        return base.to_string();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| project.kind(n).is_none()).expect("names are unbounded")
}

/// Whether `name` can name an asset: a relative `/`-separated path of plain parts.
pub fn valid_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("a name cannot be empty".into());
    }
    if name == "project" {
        return Err("\"project\" is the project's own file".into());
    }
    for part in name.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.starts_with('.') {
            return Err(format!("{name:?} is not a path inside the project"));
        }
        if part.contains(['\\', ':', '*', '?', '"', '<', '>', '|']) {
            return Err(format!("{part:?} has a character a file name cannot"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use folder::Preset;

    fn project() -> ProjectDir {
        let mut p = ProjectDir::new("t");
        pixelle_core::palette::presets::add(&mut p.palette, pixelle_core::palette::presets::DB32).unwrap();
        let mut s = SpriteFile::new(4, 3);
        s.layers.push(SpriteLayer { name: "sketch".into(), export: false, visible: false });
        let mut a = Indexed::new(4, 3);
        a.set(0, 0, 3);
        a.set(3, 2, 5);
        s.frames[0].set_layer("art", a.clone());
        let mut f = SpriteFrame::new(6, 2);
        f.name = Some("wide".into());
        f.duration = 0.1;
        f.points.insert("hand".into(), IVec2::new(1, 1));
        let mut b = Indexed::new(6, 2);
        b.set(5, 1, 9);
        f.set_layer("sketch", b);
        s.frames.push(f);
        p.sprites.insert("hero".into(), s);
        let mut t = TerrainFile::new(Preset::TwoSurface { mirror: false }, 8);
        for (i, pic) in t.tiles.iter_mut().enumerate() {
            pic.px.fill(i as u8 + 1);
        }
        t.tiles[3].set(1, 2, 30);
        p.terrains.insert("walls".into(), t);
        p
    }

    #[test]
    fn a_sprite_comes_back_off_its_canvas_as_it_went_on() {
        let p = project();
        let s = &p.sprites["hero"];
        let canvas = sprite_canvas(&p, "hero", s);
        assert_eq!(&canvas_sprite(&canvas), s);
        assert!(canvas.validate().is_empty(), "{:?}", canvas.validate());
    }

    #[test]
    fn a_terrain_comes_back_off_its_canvas_as_it_went_on() {
        let p = project();
        let t = &p.terrains["walls"];
        let canvas = terrain_canvas(&p, "walls", t);
        assert_eq!(&canvas_terrain(&canvas, t), t);
        assert!(canvas.validate().is_empty(), "{:?}", canvas.validate());
        // The patch draws with the tiles.
        let mut r = pixelle_core::render::Renderer::new(&canvas);
        let patch = patch_rect(&canvas).unwrap();
        let px = IRect::new(patch.x * 8, patch.y * 8, patch.w * 8, patch.h * 8);
        let ground = canvas.layers.iter().find(|l| l.name == "ground").unwrap().id;
        assert!(!r.indexed(px, &[ground]).is_blank());
    }

    #[test]
    fn the_sandbox_paints_with_the_projects_terrains() {
        let p = project();
        let mut canvas = sandbox_canvas(&p);
        assert_eq!(canvas.terrains.len(), 1);
        let ground = canvas.layers.iter().find(|l| l.name == "ground").unwrap().id;
        if let LayerKind::Terrain(t) = &mut canvas.layer_mut(ground).unwrap().kind {
            t.cells.set(IVec2::ZERO, 1);
        }
        let mut r = pixelle_core::render::Renderer::new(&canvas);
        assert!(!r.indexed(IRect::new(0, 0, 8, 8), &[ground]).is_blank());
    }

    #[test]
    fn frames_resize_and_keep_their_pixels() {
        let p = project();
        let canvas = sprite_canvas(&p, "hero", &p.sprites["hero"]);
        let grown = reshape_sprite(&canvas, |s| resize_frame(&mut s.frames[0], 10, 10));
        let s = canvas_sprite(&grown);
        assert_eq!((s.frames[0].w, s.frames[0].h), (10, 10));
        assert_eq!(s.frames[0].pixels["art"].get(3, 2), 5);
        // The next frame moved along rather than being swallowed.
        assert_eq!(s.frames[1].pixels["sketch"].get(5, 1), 9);
        assert_eq!(s.frames[1].name.as_deref(), Some("wide"));
    }

    #[test]
    fn names_are_paths_inside_the_project() {
        assert!(valid_name("weapons/shotgun").is_ok());
        assert!(valid_name("../x").is_err());
        assert!(valid_name("a//b").is_err());
        assert!(valid_name(".hidden").is_err());
        assert!(valid_name("project").is_err());
        assert_eq!(free_name(&project(), "hero"), "hero 2");
    }
}
