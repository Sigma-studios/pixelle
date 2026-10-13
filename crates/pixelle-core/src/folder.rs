//! A project on disk: a folder of `.pxl` text files.
//!
//! ```text
//! art/
//!   project.pxl        Project(...)  the palette, recolour slots and the list of files
//!   player.pxl         Sprite(...)   layers and frames, pixels as rows of palette indices
//!   walls.pxl          Terrain(...)  a preset and one tile per slot
//!   fx/bullet.pxl      Sprite(...)
//! ```
//!
//! Every file has the same extension; what it is comes from its first word. Each is plain RON, so
//! a change to one sprite is a change to one file, and a change to a few pixels is a change to a
//! few lines of it: a pixel is two characters, its palette index in hex, with `..` for the
//! transparent index 0 so that shapes stand out in a diff.
//!
//! The root lists every file because a game cannot always list a folder — on the web there is no
//! such thing. Nobody writes that list: it is the set of files in the project, rebuilt on save.
//!
//! An asset is named by its path without the extension: `fx/bullet.pxl` is `fx/bullet`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::d4::D4;
use crate::geom::{IRect, IVec2};
use crate::grid::Grid;
use crate::layer::LayerKind;
use crate::palette::{Palette, PaletteFull, Rgba};
use crate::project::{Project, SwapSlot};
use crate::render::Indexed;
use crate::sprite::{Frame, Sprite};
use crate::tiles::{Terrain, Tileset, presets};

pub const EXT: &str = "pxl";
/// The root file of a project, at the top of its folder.
pub const ROOT: &str = "project.pxl";
/// Bumped when a file written now would not read in an older build.
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    /// The file, relative to the project folder.
    pub file: String,
    pub message: String,
}

impl Error {
    fn new(file: impl Into<String>, message: impl std::fmt::Display) -> Self {
        Error { file: file.into(), message: message.to_string() }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.file, self.message)
    }
}

impl std::error::Error for Error {}

// --- Files ----------------------------------------------------------------------------------------

/// One `.pxl` file, of whichever kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum File {
    Project(Manifest),
    Sprite(SpriteFile),
    Terrain(TerrainFile),
}

/// The root file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub name: String,
    /// The grid the editor snaps to, in pixels.
    #[serde(default = "default_grid")]
    pub grid: u32,
    /// In index order: the first colour of the first section is index 1.
    pub palette: Vec<PaletteSection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub swaps: Vec<SwapSlot>,
    /// Every other file of the project, relative to its folder, `/`-separated.
    pub files: Vec<String>,
}

fn default_grid() -> u32 {
    8
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaletteSection {
    pub label: String,
    pub colours: Vec<(String, Hex)>,
}

/// A colour written `#rrggbb`, or `#rrggbbaa` when it is not opaque.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hex(pub Rgba);

impl Serialize for Hex {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let [r, g, b, a] = self.0;
        if a == 255 {
            s.serialize_str(&format!("#{r:02x}{g:02x}{b:02x}"))
        } else {
            s.serialize_str(&format!("#{r:02x}{g:02x}{b:02x}{a:02x}"))
        }
    }
}

impl<'de> Deserialize<'de> for Hex {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let digits = s.strip_prefix('#').unwrap_or(&s);
        let byte = |i: usize| u8::from_str_radix(digits.get(i..i + 2).unwrap_or("x"), 16);
        let bad = || serde::de::Error::custom(format!("{s:?} is not a colour; write #rrggbb or #rrggbbaa"));
        match digits.len() {
            6 | 8 => {
                let a = if digits.len() == 8 { byte(6).map_err(|_| bad())? } else { 255 };
                Ok(Hex([byte(0).map_err(|_| bad())?, byte(2).map_err(|_| bad())?, byte(4).map_err(|_| bad())?, a]))
            }
            _ => Err(bad()),
        }
    }
}

/// A sprite: a still, an animation or a sheet of frames, drawn on layers of its own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpriteFile {
    pub version: u32,
    /// Bottom to top.
    pub layers: Vec<SpriteLayer>,
    /// The origin the game positions this sprite by, in frame pixels. `None` is the centre.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot: Option<IVec2>,
    /// Free-form metadata for the game.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: BTreeMap<String, String>,
    pub frames: Vec<SpriteFrame>,
    /// Animations made of the frames, by name: `"run"` is frames 1 and 2 for 0.11 s each. The game
    /// plays one as a sprite of its own, `player#run`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub anims: BTreeMap<String, Vec<Step>>,
}

/// One step of an animation: a frame of its sprite, shown for a while. Written `(frame, seconds)`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "(usize, f32)", into = "(usize, f32)")]
pub struct Step {
    /// Which frame, by index.
    pub frame: usize,
    /// Seconds.
    pub duration: f32,
}

impl From<(usize, f32)> for Step {
    fn from((frame, duration): (usize, f32)) -> Self {
        Step { frame, duration }
    }
}

impl From<Step> for (usize, f32) {
    fn from(s: Step) -> Self {
        (s.frame, s.duration)
    }
}

/// What separates a sprite's name from one of its animations': `player#run`.
pub const ANIM_SEPARATOR: char = '#';

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpriteLayer {
    pub name: String,
    /// Part of what the game gets. A layer that is not is a guide: a sketch, notes, a mockup.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub export: bool,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub visible: bool,
}

fn yes() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpriteFrame {
    /// Code can refer to a named frame by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub w: u32,
    pub h: u32,
    /// Seconds this frame shows for in an animation; `0` for stills.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub duration: f32,
    /// Exported turned this way, e.g. a transposed copy for an engine that cannot transpose.
    #[serde(default, skip_serializing_if = "D4::is_identity")]
    pub transform: D4,
    /// Named anchor pixels, in frame coordinates.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub points: BTreeMap<String, IVec2>,
    /// Named rectangles, in frame coordinates.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rects: BTreeMap<String, IRect>,
    /// Each layer's pixels, by layer name. A layer with nothing on it in this frame is left out.
    #[serde(default)]
    pub pixels: BTreeMap<String, Indexed>,
}

impl SpriteFrame {
    pub fn new(w: u32, h: u32) -> Self {
        Self {
            name: None,
            w,
            h,
            duration: 0.0,
            transform: D4::IDENTITY,
            points: BTreeMap::new(),
            rects: BTreeMap::new(),
            pixels: BTreeMap::new(),
        }
    }

    /// A layer's pixels in this frame, blank if it has none.
    pub fn layer(&self, name: &str) -> Indexed {
        self.pixels.get(name).cloned().unwrap_or_else(|| Indexed::new(self.w, self.h))
    }

    /// Set a layer's pixels, leaving the layer out if they are blank.
    pub fn set_layer(&mut self, name: &str, pic: Indexed) {
        if pic.is_blank() {
            self.pixels.remove(name);
        } else {
            self.pixels.insert(name.into(), pic);
        }
    }
}

impl SpriteFile {
    pub fn new(w: u32, h: u32) -> Self {
        Self {
            version: VERSION,
            layers: vec![SpriteLayer { name: "art".into(), export: true, visible: true }],
            pivot: None,
            tags: BTreeMap::new(),
            frames: vec![SpriteFrame::new(w, h)],
            anims: BTreeMap::new(),
        }
    }

    /// Put a frame at `at`, keeping every animation on the frames it showed.
    pub fn insert_frame(&mut self, at: usize, frame: SpriteFrame) {
        self.frames.insert(at, frame);
        for step in self.anims.values_mut().flatten() {
            if step.frame >= at {
                step.frame += 1;
            }
        }
    }

    /// Take out the frame at `at`. Steps that showed it go, and the rest keep their frames.
    pub fn remove_frame(&mut self, at: usize) -> SpriteFrame {
        let f = self.frames.remove(at);
        for steps in self.anims.values_mut() {
            steps.retain(|s| s.frame != at);
            for step in steps {
                if step.frame > at {
                    step.frame -= 1;
                }
            }
        }
        f
    }

    /// Swap two frames, and the steps that show them.
    pub fn swap_frames(&mut self, a: usize, b: usize) {
        self.frames.swap(a, b);
        for step in self.anims.values_mut().flatten() {
            if step.frame == a {
                step.frame = b;
            } else if step.frame == b {
                step.frame = a;
            }
        }
    }

    /// Frame `i` as exported: the exported layers composited, topmost non-transparent pixel
    /// winning, then turned by the frame's transform.
    pub fn composite(&self, i: usize) -> Indexed {
        let f = &self.frames[i];
        let mut out = Indexed::new(f.w, f.h);
        for layer in self.layers.iter().filter(|l| l.export) {
            if let Some(pic) = f.pixels.get(&layer.name) {
                out.blit(pic, IVec2::ZERO);
            }
        }
        out.transformed(f.transform)
    }
}

/// Which built-in terrain a terrain file draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preset {
    /// A grid of ground cells against air. See [`presets::two_surface`]; with `every_cell` (and
    /// the mirror off), [`presets::every_cell`]: a tile for each of the 26 cells otherwise built
    /// from quarters, generated from the drawn tiles until drawn over.
    TwoSurface {
        mirror: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        every_cell: bool,
    },
}

impl Preset {
    pub fn two_surface(&self) -> presets::TwoSurface {
        match *self {
            Preset::TwoSurface { mirror: false, every_cell: true } => presets::every_cell(presets::two_surface(false)),
            Preset::TwoSurface { mirror, .. } => presets::two_surface(mirror),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Preset::TwoSurface { .. } => "2D grid, two surfaces",
        }
    }
}

/// A terrain: one of the presets, and the tiles it needs drawn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "TerrainRaw", into = "TerrainRaw")]
pub struct TerrainFile {
    pub version: u32,
    pub preset: Preset,
    /// The size of a cell, and of every tile, in pixels.
    pub cell: u32,
    /// One per slot of the preset, in its order.
    pub tiles: Vec<Indexed>,
}

impl TerrainFile {
    pub fn new(preset: Preset, cell: u32) -> Self {
        let n = preset.two_surface().slots.len();
        Self { version: VERSION, preset, cell, tiles: vec![Indexed::new(cell, cell); n] }
    }

    /// What an assembled cell (see [`Preset::TwoSurface`]) looks like when it is not drawn over:
    /// its quarters, cut from the tiles they come from. `None` for a tile that is drawn.
    pub fn generated(&self, slot: usize) -> Option<Indexed> {
        let from = self.preset.two_surface().slots.get(slot)?.assembled?;
        Some(presets::assemble(&self.tiles, from))
    }

    /// Whether an assembled cell has been drawn over, rather than following its quarters.
    pub fn drawn_over(&self, slot: usize) -> bool {
        self.generated(slot).is_some_and(|g| g != self.tiles[slot])
    }

    /// Bring every assembled cell that is not drawn over up to date with the tiles it comes from,
    /// after they changed. `keep` are the ones drawn over, which stay as they are.
    pub fn regenerate(&mut self, keep: impl Fn(usize) -> bool) {
        let slots = self.preset.two_surface().slots;
        for (i, s) in slots.iter().enumerate() {
            if let Some(from) = s.assembled
                && !keep(i)
            {
                self.tiles[i] = presets::assemble(&self.tiles, from);
            }
        }
    }

    /// Turn [`Preset::TwoSurface`]'s `every_cell` on, adding the assembled cells as generated, or
    /// off, dropping them — and with them anything drawn over them.
    pub fn set_every_cell(&mut self, on: bool) {
        let Preset::TwoSurface { mirror, every_cell } = &mut self.preset;
        if *mirror || *every_cell == on {
            return;
        }
        *every_cell = on;
        let n = self.preset.two_surface().slots.len();
        self.tiles.resize(n, Indexed::new(self.cell, self.cell));
        self.regenerate(|_| false);
    }

    /// The terrain's rules, pointing at `tiles` by index.
    pub fn terrain(&self, name: &str) -> Terrain {
        let ts = self.preset.two_surface();
        Terrain { name: name.into(), tileset: name.into(), rules: ts.rules, fallback: None, quarters: ts.quarters }
    }
}

/// How a terrain file is written: its tiles as a map from slot name to rows, in slot order.
#[derive(Serialize, Deserialize)]
#[serde(rename = "TerrainFile")]
struct TerrainRaw {
    version: u32,
    preset: Preset,
    cell: u32,
    tiles: Ordered<Indexed>,
}

impl From<TerrainFile> for TerrainRaw {
    fn from(t: TerrainFile) -> Self {
        let slots = t.preset.two_surface().slots;
        // An assembled cell is written only when drawn over: otherwise it is its quarters, and
        // follows them when they change.
        let tiles = (0..slots.len())
            .filter(|&i| slots[i].assembled.is_none() || t.drawn_over(i))
            .map(|i| (slots[i].name.clone(), t.tiles[i].clone()))
            .collect();
        TerrainRaw { version: t.version, preset: t.preset, cell: t.cell, tiles: Ordered(tiles) }
    }
}

impl TryFrom<TerrainRaw> for TerrainFile {
    type Error = String;
    fn try_from(raw: TerrainRaw) -> Result<Self, String> {
        let slots = raw.preset.two_surface().slots;
        let mut tiles = vec![Indexed::new(raw.cell, raw.cell); slots.len()];
        let mut given = vec![false; slots.len()];
        for (name, pic) in raw.tiles.0 {
            let Some(i) = slots.iter().position(|s| s.name == name) else {
                let known: Vec<&str> = slots.iter().map(|s| s.name.as_str()).collect();
                return Err(format!("{} has no tile {name:?}; its tiles are {known:?}", raw.preset.label()));
            };
            if (pic.w, pic.h) != (raw.cell, raw.cell) {
                return Err(format!("tile {name:?} is {}x{}, but cells are {}", pic.w, pic.h, raw.cell));
            }
            tiles[i] = pic;
            given[i] = true;
        }
        let mut t = TerrainFile { version: raw.version, preset: raw.preset, cell: raw.cell, tiles };
        t.regenerate(|i| given[i]);
        Ok(t)
    }
}

/// A map written in the order given rather than sorted.
struct Ordered<T>(Vec<(String, T)>);

impl<T: Serialize> Serialize for Ordered<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            m.serialize_entry(k, v)?;
        }
        m.end()
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Ordered<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for V<T> {
            type Value = Ordered<T>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "a map")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    out.push(entry);
                }
                Ok(Ordered(out))
            }
        }
        d.deserialize_map(V(std::marker::PhantomData))
    }
}

/// A picture is written as rows of two-character palette indices: `..` for 0, hex otherwise.
impl Serialize for Indexed {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = s.serialize_seq(Some(self.h as usize))?;
        for y in 0..self.h {
            let mut row = String::with_capacity(self.w as usize * 2);
            for x in 0..self.w {
                match self.get(x, y) {
                    0 => row.push_str(".."),
                    v => row.push_str(&format!("{v:02x}")),
                }
            }
            seq.serialize_element(&row)?;
        }
        seq.end()
    }
}

impl<'de> Deserialize<'de> for Indexed {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let rows: Vec<String> = Vec::deserialize(d)?;
        parse_rows(&rows).map_err(serde::de::Error::custom)
    }
}

/// Rows of two-character pixels into a picture.
pub fn parse_rows(rows: &[String]) -> Result<Indexed, String> {
    let w = rows.first().map_or(0, |r| r.len());
    if w % 2 != 0 {
        return Err(format!("row 0 has {w} characters; a pixel is two"));
    }
    let mut out = Indexed::new(w as u32 / 2, rows.len() as u32);
    for (y, row) in rows.iter().enumerate() {
        if row.len() != w {
            return Err(format!("row {y} is {} characters long, row 0 is {w}", row.len()));
        }
        for (x, pair) in row.as_bytes().chunks(2).enumerate() {
            let v = match pair {
                b".." => 0,
                _ => std::str::from_utf8(pair)
                    .ok()
                    .and_then(|p| u8::from_str_radix(p, 16).ok())
                    .ok_or_else(|| format!("row {y}, pixel {x}: {:?} is not a palette index", String::from_utf8_lossy(pair)))?,
            };
            out.set(x as u32, y as u32, v);
        }
    }
    Ok(out)
}

fn ron_options() -> ron::Options {
    ron::Options::default().with_default_extension(ron::extensions::Extensions::UNWRAP_VARIANT_NEWTYPES)
}

fn pretty() -> ron::ser::PrettyConfig {
    ron::ser::PrettyConfig::new()
        .indentor("  ".to_string())
        .struct_names(false)
        .extensions(ron::extensions::Extensions::UNWRAP_VARIANT_NEWTYPES)
}

impl File {
    pub fn to_text(&self) -> String {
        let mut s = ron_options().to_string_pretty(self, pretty()).expect("a project file always writes");
        s.push('\n');
        s
    }

    pub fn from_text(file: &str, text: &str) -> Result<File, Error> {
        let parsed: File = ron_options().from_str(text).map_err(|e| {
            // A newer file would fail somewhere odd; say why instead.
            if let Some(v) = sniff_version(text)
                && v > VERSION
            {
                return Error::new(file, format!("written by a newer Pixelle (format {v}; this one reads {VERSION})"));
            }
            Error::new(file, e)
        })?;
        let version = match &parsed {
            File::Project(m) => m.version,
            File::Sprite(s) => s.version,
            File::Terrain(t) => t.version,
        };
        if version > VERSION {
            return Err(Error::new(file, format!("written by a newer Pixelle (format {version}; this one reads {VERSION})")));
        }
        Ok(parsed)
    }

    pub fn from_bytes(file: &str, bytes: &[u8]) -> Result<File, Error> {
        if bytes.starts_with(b"PXLL") {
            return Err(Error::new(file, "an old single-file project; this Pixelle opens project folders"));
        }
        let text = std::str::from_utf8(bytes).map_err(|_| Error::new(file, "not a text file"))?;
        File::from_text(file, text)
    }

    pub fn kind(&self) -> &'static str {
        match self {
            File::Project(_) => "project",
            File::Sprite(_) => "sprite",
            File::Terrain(_) => "terrain",
        }
    }
}

fn sniff_version(text: &str) -> Option<u32> {
    let at = text.find("version:")? + "version:".len();
    text[at..].trim_start().split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

// --- The project ----------------------------------------------------------------------------------

/// A whole project in memory: the root's palette and slots, and every file it lists.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectDir {
    pub name: String,
    pub grid: u32,
    pub palette: Palette,
    pub swaps: Vec<SwapSlot>,
    /// By asset name.
    pub sprites: BTreeMap<String, SpriteFile>,
    pub terrains: BTreeMap<String, TerrainFile>,
}

/// What an asset is, when only its name is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Sprite,
    Terrain,
}

/// The asset a file holds: its path without the extension.
pub fn asset_name(rel_path: &str) -> String {
    rel_path.strip_suffix(".pxl").unwrap_or(rel_path).to_string()
}

pub fn file_name(asset: &str) -> String {
    format!("{asset}.{EXT}")
}

impl ProjectDir {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            grid: 8,
            palette: Palette::default(),
            swaps: Vec::new(),
            sprites: BTreeMap::new(),
            terrains: BTreeMap::new(),
        }
    }

    pub fn kind(&self, asset: &str) -> Option<Kind> {
        if self.sprites.contains_key(asset) {
            Some(Kind::Sprite)
        } else if self.terrains.contains_key(asset) {
            Some(Kind::Terrain)
        } else {
            None
        }
    }

    /// Every asset name, sorted.
    pub fn assets(&self) -> Vec<String> {
        let mut out: Vec<String> = self.sprites.keys().chain(self.terrains.keys()).cloned().collect();
        out.sort();
        out
    }

    pub fn manifest(&self) -> Manifest {
        let mut palette = Vec::new();
        let entries = self.palette.entries();
        let mut covered = vec![false; entries.len()];
        for s in self.palette.sections() {
            palette.push(PaletteSection {
                label: s.label.clone(),
                colours: s
                    .indices()
                    .map(|i| {
                        covered[i as usize] = true;
                        let e = &entries[i as usize];
                        (e.name.clone(), Hex(e.rgba))
                    })
                    .collect(),
            });
        }
        // Sections are contiguous runs; an entry outside every one keeps its place in one of its own.
        debug_assert!(covered.iter().skip(1).all(|c| *c), "every palette entry belongs to a section");
        Manifest {
            version: VERSION,
            name: self.name.clone(),
            grid: self.grid,
            palette,
            swaps: self.swaps.clone(),
            files: self.assets().iter().map(|a| file_name(a)).collect(),
        }
    }

    /// Every file of the project as text, by path relative to its folder, the root included.
    pub fn to_files(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        out.insert(ROOT.to_string(), File::Project(self.manifest()).to_text());
        for (name, s) in &self.sprites {
            out.insert(file_name(name), File::Sprite(s.clone()).to_text());
        }
        for (name, t) in &self.terrains {
            out.insert(file_name(name), File::Terrain(t.clone()).to_text());
        }
        out
    }

    /// One asset's file as text.
    pub fn file_text(&self, asset: &str) -> Option<String> {
        if let Some(s) = self.sprites.get(asset) {
            return Some(File::Sprite(s.clone()).to_text());
        }
        self.terrains.get(asset).map(|t| File::Terrain(t.clone()).to_text())
    }

    pub fn from_manifest(m: Manifest) -> Result<Self, Error> {
        let mut palette = Palette::default();
        for s in &m.palette {
            palette.begin_section(&s.label);
            for (name, Hex(rgba)) in &s.colours {
                palette.push(name, *rgba).map_err(|e: PaletteFull| Error::new(ROOT, e))?;
            }
        }
        Ok(Self {
            name: m.name,
            grid: m.grid,
            palette,
            swaps: m.swaps,
            sprites: BTreeMap::new(),
            terrains: BTreeMap::new(),
        })
    }

    /// Put a file's contents in the project as `asset`.
    pub fn insert(&mut self, asset: &str, file: File) -> Result<(), Error> {
        match file {
            File::Sprite(s) => {
                self.terrains.remove(asset);
                self.sprites.insert(asset.into(), s);
            }
            File::Terrain(t) => {
                self.sprites.remove(asset);
                self.terrains.insert(asset.into(), t);
            }
            File::Project(_) => return Err(Error::new(file_name(asset), "a project root inside a project")),
        }
        Ok(())
    }

    pub fn remove(&mut self, asset: &str) {
        self.sprites.remove(asset);
        self.terrains.remove(asset);
    }

    /// Read a project through `read`, which is given each path relative to the project folder:
    /// the root first, then every file it lists.
    pub fn load_with(mut read: impl FnMut(&str) -> Result<Vec<u8>, String>) -> Result<Self, Error> {
        let root = read(ROOT).map_err(|e| Error::new(ROOT, e))?;
        let File::Project(manifest) = File::from_bytes(ROOT, &root)? else {
            return Err(Error::new(ROOT, "is not a project root"));
        };
        if manifest.version > VERSION {
            return Err(Error::new(ROOT, format!("written by a newer Pixelle (format {})", manifest.version)));
        }
        let files = manifest.files.clone();
        let mut project = Self::from_manifest(manifest)?;
        for path in files {
            let bytes = read(&path).map_err(|e| Error::new(&path, e))?;
            let file = File::from_bytes(&path, &bytes)?;
            project.insert(&asset_name(&path), file).map_err(|e| Error::new(&path, e.message))?;
        }
        Ok(project)
    }

    /// Read a project folder.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self, Error> {
        let dir = dir.as_ref();
        Self::load_with(|rel| std::fs::read(dir.join(rel)).map_err(|e| e.to_string()))
    }

    /// Write the project into `dir`, touching only files whose text changed. Returns the files
    /// written. Files of assets no longer in the project are left alone: deleting is up to the
    /// caller, who knows it was meant.
    pub fn save(&self, dir: impl AsRef<Path>) -> Result<Vec<String>, Error> {
        let dir = dir.as_ref();
        let mut written = Vec::new();
        for (rel, text) in self.to_files() {
            let path = dir.join(&rel);
            if std::fs::read(&path).is_ok_and(|old| old == text.as_bytes()) {
                continue;
            }
            write_atomic(&path, text.as_bytes()).map_err(|e| Error::new(&rel, e))?;
            written.push(rel);
        }
        Ok(written)
    }

    /// Add a colour at the end of a palette section, remapping every pixel and slot after it.
    pub fn insert_color(&mut self, section: usize, name: impl Into<String>, rgba: Rgba) -> Result<u8, PaletteFull> {
        let s = &self.palette.sections()[section];
        let at = (s.start + s.len) as u8;
        if self.palette.len() >= 256 {
            return Err(PaletteFull);
        }
        if (at as usize) < self.palette.len() {
            self.remap_indices(|v| if v >= at && v != 0 { v + 1 } else { v });
        }
        self.palette.insert_into_section(section, name.into(), rgba)
    }

    /// Remove a colour. Pixels of it become `replace_with` (an index as it is now), and later
    /// indices shift down.
    pub fn remove_color(&mut self, index: u8, replace_with: u8) {
        if index == 0 {
            return;
        }
        let replace = if replace_with == index {
            0
        } else if replace_with > index {
            replace_with - 1
        } else {
            replace_with
        };
        self.remap_indices(|v| match v {
            v if v == index => replace,
            v if v > index => v - 1,
            v => v,
        });
        self.swaps.retain(|s| s.index != index);
        self.palette.remove(index);
    }

    /// Run every palette index in the project through `f`. Returns the assets whose pixels
    /// changed.
    pub fn remap_indices(&mut self, f: impl Fn(u8) -> u8) -> Vec<String> {
        let mut changed = Vec::new();
        let remap = |pic: &mut Indexed| {
            let mut any = false;
            for v in &mut pic.px {
                let n = f(*v);
                any |= n != *v;
                *v = n;
            }
            any
        };
        for (name, s) in &mut self.sprites {
            let mut any = false;
            for frame in &mut s.frames {
                for pic in frame.pixels.values_mut() {
                    any |= remap(pic);
                }
            }
            if any {
                changed.push(name.clone());
            }
        }
        for (name, t) in &mut self.terrains {
            let mut any = false;
            for pic in &mut t.tiles {
                any |= remap(pic);
            }
            if any {
                changed.push(name.clone());
            }
        }
        for s in &mut self.swaps {
            s.index = f(s.index);
        }
        changed
    }

    /// How many pixels in the project hold this index.
    pub fn color_usage(&self, index: u8) -> usize {
        let count = |pic: &Indexed| pic.px.iter().filter(|v| **v == index).count();
        self.sprites.values().flat_map(|s| &s.frames).flat_map(|f| f.pixels.values()).map(count).sum::<usize>()
            + self.terrains.values().flat_map(|t| &t.tiles).map(count).sum::<usize>()
    }

    /// Problems that would make the game's art wrong. Empty is good.
    pub fn validate(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (name, s) in &self.sprites {
            if s.frames.is_empty() {
                out.push(format!("{name}: no frames"));
            }
            let mut layer_names = std::collections::HashSet::new();
            for l in &s.layers {
                if !layer_names.insert(&l.name) {
                    out.push(format!("{name}: two layers called {:?}", l.name));
                }
            }
            let mut frame_names = std::collections::HashSet::new();
            for (i, f) in s.frames.iter().enumerate() {
                if let Some(n) = &f.name
                    && !frame_names.insert(n)
                {
                    out.push(format!("{name}: two frames called {n:?}"));
                }
                for (layer, pic) in &f.pixels {
                    if !layer_names.contains(layer) {
                        out.push(format!("{name} frame {i}: pixels on missing layer {layer:?}"));
                    }
                    if (pic.w, pic.h) != (f.w, f.h) {
                        out.push(format!("{name} frame {i}: layer {layer:?} is {}x{}, the frame is {}x{}", pic.w, pic.h, f.w, f.h));
                    }
                    if let Some(v) = pic.px.iter().find(|v| **v as usize >= self.palette.len()) {
                        out.push(format!("{name} frame {i}: layer {layer:?} uses index {v:02x}, past the palette"));
                    }
                }
                for (p, at) in &f.points {
                    // Points may sit on the far edge too: some mark a corner between pixels.
                    if at.x < 0 || at.y < 0 || at.x > f.w as i32 || at.y > f.h as i32 {
                        out.push(format!("{name} frame {i}: point {p:?} {at} is outside the frame"));
                    }
                }
            }
        }
        for (name, s) in &self.sprites {
            for (anim, steps) in &s.anims {
                if anim.is_empty() || anim.contains([ANIM_SEPARATOR, '/', ',']) {
                    out.push(format!("{name}: an animation cannot be called {anim:?}"));
                }
                if steps.is_empty() {
                    out.push(format!("{name}: animation {anim:?} has no steps"));
                }
                for step in steps {
                    if step.frame >= s.frames.len() {
                        out.push(format!("{name}: animation {anim:?} shows frame {}, of {}", step.frame, s.frames.len()));
                    }
                }
            }
        }
        for (name, t) in &self.terrains {
            if t.cell == 0 || t.cell % 2 != 0 {
                out.push(format!("{name}: cells of {} pixels cannot be split in quarters", t.cell));
            }
            if t.tiles.len() != t.preset.two_surface().slots.len() {
                out.push(format!("{name}: {} tiles for {} slots", t.tiles.len(), t.preset.two_surface().slots.len()));
            }
        }
        for swap in &self.swaps {
            if self.palette.section(&swap.options).is_none() {
                out.push(format!("swap slot {:?} picks from missing section {:?}", swap.name, swap.options));
            }
        }
        out
    }

    /// The project laid out on one canvas — every sprite's frames in a row, every terrain's tiles
    /// in a row beneath them — which is what compositing, packing and baking work from.
    pub fn to_canvas(&self) -> Project {
        const GAP: i32 = 2;
        let mut p = Project::new(self.name.clone());
        p.grid = self.grid;
        p.palette = self.palette.clone();
        p.swaps = self.swaps.clone();
        let mut y = 0;
        for (name, s) in &self.sprites {
            let mut ids = Vec::new();
            let mut exported = Vec::new();
            for l in &s.layers {
                let id = p.add_layer(format!("{name}/{}", l.name), LayerKind::Pixels(Grid::new()));
                let layer = p.layer_mut(id).expect("just added");
                layer.visible = l.visible;
                layer.reference = !l.export;
                ids.push((l.name.clone(), id));
                if l.export {
                    exported.push(id);
                }
            }
            let mut x = 0;
            let mut frames = Vec::new();
            let mut tall = 0;
            for f in &s.frames {
                for (layer, id) in &ids {
                    if let Some(pic) = f.pixels.get(layer) {
                        let grid = p.layer_mut(*id).and_then(|l| l.pixels_mut()).expect("a pixel layer");
                        paint(grid, pic, IVec2::new(x, y));
                    }
                }
                frames.push(Frame {
                    rect: IRect::new(x, y, f.w, f.h),
                    transform: f.transform,
                    duration: f.duration,
                    points: f.points.clone(),
                    rects: f.rects.clone(),
                });
                x += f.w as i32 + GAP;
                tall = tall.max(f.h as i32);
            }
            let mut sprite = Sprite::new(name.clone(), exported, frames);
            sprite.pivot = s.pivot;
            sprite.tags = s.tags.clone();
            if s.frames.iter().any(|f| f.name.is_some()) {
                let names: Vec<String> =
                    s.frames.iter().enumerate().map(|(i, f)| f.name.clone().unwrap_or_else(|| format!("frame_{i}"))).collect();
                sprite.tags.insert("frames".into(), names.join(", "));
            }
            let sheet_frames = sprite.frames.clone();
            let exported = sprite.layers.clone();
            p.sprites.push(sprite);
            for anim in anim_sprites(name, s, &sheet_frames, &exported) {
                p.sprites.push(anim);
            }
            y += tall + GAP;
        }
        for (name, t) in &self.terrains {
            let mut grid = Grid::new();
            for (i, pic) in t.tiles.iter().enumerate() {
                paint(&mut grid, pic, IVec2::new(i as i32 * t.cell as i32, y));
            }
            let id = p.add_layer(format!("{name}/tiles"), LayerKind::Pixels(grid));
            p.tilesets.push(Tileset {
                name: name.clone(),
                origin: IVec2::new(0, y),
                tile_size: t.cell,
                columns: t.tiles.len().max(1) as u32,
                count: t.tiles.len() as u32,
                layers: vec![id],
                gap: 0,
            });
            p.terrains.push(t.terrain(name));
            y += t.cell as i32 + GAP;
        }
        p
    }
}

/// A sprite's animations as sprites of their own on its canvas: each step the rectangle of the
/// frame it shows, with the step's duration. Named `sprite#anim`, and tagged with the sprite.
pub fn anim_sprites(name: &str, s: &SpriteFile, sheet_frames: &[Frame], layers: &[crate::layer::LayerId]) -> Vec<Sprite> {
    s.anims
        .iter()
        .map(|(anim, steps)| {
            let frames = steps
                .iter()
                .filter_map(|step| sheet_frames.get(step.frame).map(|f| Frame { duration: step.duration, ..f.clone() }))
                .collect();
            let mut sprite = Sprite::new(format!("{name}{ANIM_SEPARATOR}{anim}"), layers.to_vec(), frames);
            sprite.pivot = s.pivot;
            sprite.tags.insert(ANIM_OF.into(), name.into());
            let shown: Vec<String> = steps.iter().filter(|s| s.frame < sheet_frames.len()).map(|s| s.frame.to_string()).collect();
            sprite.tags.insert(ANIM_FRAMES.into(), shown.join(","));
            sprite
        })
        .collect()
}

/// The tag an animation's sprite carries: which sprite it is an animation of.
pub const ANIM_OF: &str = "anim_of";

/// The tag an animation's sprite carries: which frame of that sprite each of its frames is,
/// comma-separated. For code that draws the sheet itself and wants the animation's timing.
pub const ANIM_FRAMES: &str = "anim_frames";

fn paint(grid: &mut Grid, pic: &Indexed, at: IVec2) {
    for yy in 0..pic.h {
        for xx in 0..pic.w {
            let v = pic.get(xx, yy);
            if v != 0 {
                grid.set(at + IVec2::new(xx as i32, yy as i32), v);
            }
        }
    }
}

/// Write beside and rename, so a crash mid-save never leaves half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = PathBuf::from(path);
    tmp.as_mut_os_string().push(".tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ProjectDir {
        let mut p = ProjectDir::new("t");
        crate::palette::presets::add(&mut p.palette, crate::palette::presets::PICO8).unwrap();
        p.palette.begin_section("Mine");
        p.palette.push("glass", [10, 20, 30, 128]).unwrap();
        let mut s = SpriteFile::new(4, 3);
        s.layers.push(SpriteLayer { name: "sketch".into(), export: false, visible: false });
        let mut pic = Indexed::new(4, 3);
        pic.set(1, 0, 3);
        pic.set(2, 2, 17);
        s.frames[0].set_layer("art", pic);
        s.frames[0].points.insert("hand".into(), IVec2::new(2, 1));
        let mut f = SpriteFrame::new(4, 3);
        f.duration = 0.11;
        f.name = Some("run".into());
        s.frames.push(f);
        s.anims.insert("blink".into(), vec![Step { frame: 0, duration: 0.5 }, Step { frame: 1, duration: 0.1 }]);
        p.sprites.insert("player/idle".into(), s);
        let mut t = TerrainFile::new(Preset::TwoSurface { mirror: false, every_cell: false }, 8);
        t.tiles[0].set(0, 0, 5);
        p.terrains.insert("walls".into(), t);
        p
    }

    /// Each of the 256 neighbourhoods of a project's `walls`, drawn as the bake draws it.
    fn every_neighbourhood(p: &ProjectDir) -> Vec<Indexed> {
        let baked = crate::bake::bake_folder(p);
        let terrain = baked.terrain("walls").expect("walls");
        let atlas = Indexed { w: baked.atlas_width, h: baked.atlas_height, px: baked.atlas.clone() };
        (0..=255u8)
            .map(|mask| {
                let same = |d: IVec2| crate::tiles::NEIGHBOURS.iter().position(|n| *n == d).is_none_or(|s| mask & (1 << s) != 0);
                let mut out = Indexed::new(8, 8);
                for piece in terrain.cell(same) {
                    let src = atlas.crop(piece.source.x as u32, piece.source.y as u32, piece.source.w, piece.source.h);
                    out.blit(&src.transformed(crate::D4::new(false, piece.flip_x, piece.flip_y)), piece.at.min());
                }
                out
            })
            .collect()
    }

    /// Walls whose every tile is different noise, so any pixel out of place shows.
    fn noisy_walls() -> ProjectDir {
        let mut p = ProjectDir::new("t");
        crate::palette::presets::add(&mut p.palette, crate::palette::presets::PICO8).unwrap();
        let mut t = TerrainFile::new(Preset::TwoSurface { mirror: false, every_cell: false }, 8);
        let mut seed = 7u32;
        for tile in &mut t.tiles {
            for y in 0..8 {
                for x in 0..8 {
                    seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    tile.set(x, y, 1 + (seed >> 16) as u8 % 15);
                }
            }
        }
        p.terrains.insert("walls".into(), t);
        p
    }

    #[test]
    fn every_cell_draws_what_the_quarters_did_until_drawn_over() {
        let mut p = noisy_walls();
        let before = every_neighbourhood(&p);
        let walls = p.terrains.get_mut("walls").unwrap();
        walls.set_every_cell(true);
        assert_eq!(walls.tiles.len(), 47);
        let names: std::collections::BTreeSet<_> = walls.preset.two_surface().slots.into_iter().map(|s| s.name).collect();
        assert_eq!(names.len(), 47, "every cell has a name of its own");
        assert_eq!(every_neighbourhood(&p), before, "generated cells are their quarters");
        // Only whole tiles now: one piece a cell.
        let baked = crate::bake::bake_folder(&p);
        let terrain = baked.terrain("walls").unwrap();
        assert!((0..=255u8).all(|m| {
            let same = |d: IVec2| crate::tiles::NEIGHBOURS.iter().position(|n| *n == d).is_none_or(|s| m & (1 << s) != 0);
            terrain.cell(same).len() == 1
        }));

        // Nothing generated is written, and it comes back.
        let text = File::Terrain(p.terrains["walls"].clone()).to_text();
        assert!(text.contains("every_cell: true") && !text.contains("inside top-left +"), "{text}");
        let File::Terrain(back) = File::from_text("walls.pxl", &text).unwrap() else { panic!() };
        assert_eq!(back, p.terrains["walls"]);

        // Drawn over: written, kept when its sources change; the others follow them.
        let walls = p.terrains.get_mut("walls").unwrap();
        let slots = walls.preset.two_surface().slots;
        let over = slots.iter().position(|s| s.assembled.is_some()).unwrap();
        let follows = slots.iter().rposition(|s| s.assembled.is_some()).unwrap();
        walls.tiles[over].set(0, 0, 0);
        let from = slots[follows].assembled.unwrap()[0] as usize;
        walls.tiles[from].set(1, 1, 0);
        walls.regenerate(|i| i == over);
        assert!(walls.drawn_over(over) && !walls.drawn_over(follows));
        assert_eq!(walls.tiles[follows].get(1, 1), 0, "a generated cell follows its quarter");
        let text = File::Terrain(walls.clone()).to_text();
        assert!(text.contains(&format!("{:?}", slots[over].name)) && !text.contains(&format!("{:?}", slots[follows].name)));
        let File::Terrain(back) = File::from_text("walls.pxl", &text).unwrap() else { panic!() };
        assert_eq!(&back, walls);

        walls.set_every_cell(false);
        assert_eq!(walls.tiles.len(), 21);
    }

    #[test]
    fn rows_read_back() {
        let mut pic = Indexed::new(3, 2);
        pic.set(0, 0, 0xab);
        pic.set(2, 1, 1);
        let text = ron::to_string(&pic).unwrap();
        assert_eq!(text, r#"["ab....","....01"]"#);
        assert_eq!(ron::from_str::<Indexed>(&text).unwrap(), pic);
        assert!(parse_rows(&["..zz".into()]).unwrap_err().contains("pixel 1"));
        assert!(parse_rows(&["....".into(), "..".into()]).unwrap_err().contains("row 1"));
    }

    #[test]
    fn a_project_writes_reads_and_writes_the_same() {
        let p = sample();
        let files = p.to_files();
        let back = ProjectDir::load_with(|rel| files.get(rel).map(|t| t.clone().into_bytes()).ok_or("missing".into())).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.to_files(), files);
        assert!(files[ROOT].starts_with("Project("), "{}", files[ROOT]);
        assert!(files["player/idle.pxl"].starts_with("Sprite("));
        assert!(files["walls.pxl"].starts_with("Terrain("));
        assert!(files[ROOT].contains("\"#0a141e80\""));
        assert!(files["player/idle.pxl"].contains("\"..03....\""), "{}", files["player/idle.pxl"]);
        assert!(p.validate().is_empty(), "{:?}", p.validate());
    }

    #[test]
    fn saving_writes_only_what_changed() {
        let dir = std::env::temp_dir().join(format!("pixelle-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut p = sample();
        assert_eq!(p.save(&dir).unwrap().len(), 3);
        assert!(p.save(&dir).unwrap().is_empty());
        p.sprites.get_mut("player/idle").unwrap().frames[1].duration = 0.2;
        assert_eq!(p.save(&dir).unwrap(), vec!["player/idle.pxl".to_string()]);
        assert_eq!(ProjectDir::load(&dir).unwrap(), p);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn palette_edits_reach_every_file() {
        let mut p = sample();
        let before = p.to_canvas();
        let at = p.insert_color(0, "new", [1, 2, 3, 255]).unwrap();
        assert_eq!(at, 17);
        let pic = &p.sprites["player/idle"].frames[0].pixels["art"];
        assert_eq!(p.palette.rgba(pic.get(1, 0)), before.palette.rgba(3));
        assert_eq!(p.palette.rgba(pic.get(2, 2)), before.palette.rgba(17));
        p.remove_color(at, 0);
        assert_eq!(p.palette, before.palette);
    }

    #[test]
    fn old_and_newer_files_say_so() {
        assert!(File::from_bytes("a.pxl", b"PXLL\x02\0").unwrap_err().message.contains("old single-file"));
        let e = File::from_text("a.pxl", "Sprite(version: 9, layers: [], frames: [], wat: 1)").unwrap_err();
        assert!(e.message.contains("newer"), "{e}");
    }

    #[test]
    fn the_canvas_exports_what_the_files_hold() {
        let p = sample();
        let canvas = p.to_canvas();
        let mut r = crate::render::Renderer::new(&canvas);
        assert_eq!(r.frame("player/idle", 0).unwrap(), p.sprites["player/idle"].composite(0));
        assert_eq!(canvas.sprite("player/idle").unwrap().tags["frames"], "frame_0, run");
        assert!(canvas.validate().is_empty(), "{:?}", canvas.validate());
    }

    #[test]
    fn animations_bake_as_sprites_of_their_own() {
        let p = sample();
        let baked = crate::bake::bake_folder(&p);
        let sheet = baked.sprite("player/idle").unwrap();
        let blink = baked.sprite("player/idle#blink").unwrap();
        assert!(blink.is_animation());
        assert_eq!(blink.frames[0].rect, sheet.frames[0].rect, "a step shares its frame's place in the atlas");
        assert_eq!(blink.frames[1].duration, 0.1);
        assert_eq!(blink.tags[ANIM_OF], "player/idle");
    }

    #[test]
    fn frame_edits_keep_animations_on_their_frames() {
        let mut s = sample().sprites["player/idle"].clone();
        s.insert_frame(0, SpriteFrame::new(4, 3));
        assert_eq!(s.anims["blink"].iter().map(|x| x.frame).collect::<Vec<_>>(), [1, 2]);
        s.swap_frames(1, 2);
        assert_eq!(s.anims["blink"].iter().map(|x| x.frame).collect::<Vec<_>>(), [2, 1]);
        s.remove_frame(1);
        assert_eq!(s.anims["blink"].iter().map(|x| x.frame).collect::<Vec<_>>(), [1]);
    }
}
