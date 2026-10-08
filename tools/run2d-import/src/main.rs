//! Turn run-2d's art into a Pixelle project folder.
//!
//! ```text
//! cargo run -p run2d-import -- /path/to/run-2d -o /path/to/run-2d/assets/art
//! cargo run -p run2d-import -- /path/to/run-2d -o examples/showcase/assets/run2d --showcase
//! ```
//!
//! What goes in:
//! - a sprite per sheet, frames in sheet order so the game's frame indices still hold, with any
//!   hidden `.aseprite` layers kept as layers of their own that are not exported, and the
//!   unexported `walls2.aseprite` as work in progress;
//! - the walls as a terrain, "2D grid, two surfaces" with the mirror off: every way a tile faces is
//!   drawn out, starting from the seven frames `cladding.rs` turns to fit;
//! - the cycles the game plays out of its sheets in code — running, crouch-walking, a shell, a
//!   rocket — as animations of those sheets, with the game's timings;
//! - every colour the game's code names, as labelled palette sections.
//!
//! With `--showcase`, also what the showcase demonstrates and run-2d does not read from art yet:
//! the pixels the code seats weapons by, the ragdoll and face cuts, the recolour slots, and the
//! bullet and blast the game draws in code.
//!
//! Before writing, it checks that every sheet comes back out of the project pixel for pixel, and
//! that the walls terrain draws every neighbourhood exactly as `cladding.rs` does.

mod colors;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use pixelle_core::aseprite;
use pixelle_core::bake;
use pixelle_core::folder::{Preset, ProjectDir, SpriteFile, SpriteFrame, SpriteLayer, Step, TerrainFile};
use pixelle_core::render::{Indexed, RgbaImage};
use pixelle_core::tiles::presets;
use pixelle_core::{IRect, IVec2, SwapSlot, png_io};

use colors::*;

/// Every frame in run-2d is this big (`atlas::FRAME`), and so is a level cell (`arena::CELL`).
const TILE: u32 = 8;

/// `main.rs:164`.
const TICK: f32 = 1.0 / 64.0;

/// `player.rs:31-46`, in sheet order.
const POSES: [&str; 10] = [
    "idle",
    "run_1",
    "run_2",
    "slide",
    "jump",
    "jump_armed",
    "wall_slide",
    "wall_slide_armed",
    "crouch",
    "crouch_walk",
];

/// `weapon.rs:632-674`, in sheet order: name, whether it is held (seated by `trigger`), whether it
/// shoots (has a `muzzle`), and `weapon.rs` `ART_OFFSET` (y up) for when it lies loose.
const WEAPONS: [(&str, bool, bool, [i32; 2]); 16] = [
    ("shotgun", true, true, [0, 0]),
    ("handgun", true, true, [0, 0]),
    ("grenade_launcher", true, true, [1, 0]),
    ("shell_1", false, false, [0, 0]),
    ("shell_2", false, false, [0, 0]),
    ("grenade_capped", true, false, [1, 0]),
    ("grenade_pulled", true, false, [1, 0]),
    ("rocket_launcher", true, true, [0, 0]),
    ("rocket_1", false, false, [0, 0]),
    ("rocket_2", false, false, [0, 0]),
    ("sword", true, false, [1, 1]),
    ("net_gun", true, true, [0, 0]),
    ("net", false, false, [0, 0]),
    ("flag_red", true, false, [2, 1]),
    ("flag_blue", true, false, [2, 1]),
    ("crate", false, false, [0, 0]),
];

/// `props.rs:61-68` and `portal.rs:43-45`.
const PROPS: [&str; 7] = ["weapon_mark", "jump_pad_rest", "jump_pad_sprung", "spikes", "portal_single", "portal_end", "portal_middle"];
/// `cladding.rs`: the caps and the middle a run of platform is drawn from.
const PLATFORMS: [&str; 3] = ["cap_left", "middle", "cap_right"];

/// `weapon.rs` `hand_pixel`: where each pose holds a weapon.
fn hand_pixel(pose: usize) -> IVec2 {
    match POSES[pose] {
        "slide" => IVec2::new(5, 6),
        "jump_armed" => IVec2::new(5, 3),
        "wall_slide_armed" => IVec2::new(5, 4),
        "crouch" => IVec2::new(6, 6),
        "crouch_walk" => IVec2::new(5, 6),
        "run_1" => IVec2::new(6, 4),
        _ => IVec2::new(5, 4),
    }
}

/// `ragdoll.rs` `cut_of`: `[left, top, right, bottom]` of each piece in the standing frame.
const RAGDOLL_CUTS: [(&str, [i32; 4]); 6] = [
    ("head", [3, 0, 6, 2]),
    ("torso", [3, 2, 5, 5]),
    ("front_arm", [5, 2, 6, 5]),
    ("back_arm", [2, 2, 3, 5]),
    ("front_leg", [4, 5, 5, 8]),
    ("back_leg", [3, 5, 4, 8]),
];

/// `weapon.rs:507-509`.
const TRIGGER: IVec2 = IVec2::new(4, 4);
const MUZZLE: IVec2 = IVec2::new(6, 3);
/// `player.rs`: seconds between the frames of the run and crouch-walk cycles.
const RUN_FRAME_TIME: f32 = 0.11;
/// `weapon.rs`: a shell (and a rocket) shows its first frame this many ticks, then its second
/// for the other.
const SHELL_RING_TICKS: f32 = 8.0;
const SHELL_CORE_TICKS: f32 = 4.0;
/// `weapon.rs:457-458`.
const EXPLOSION_TICKS: u32 = 8;
const EXPLOSION_RADIUS: f32 = 18.0;
/// The blast's frames: wide enough for its widest, 18px radius.
const BLAST: u32 = 37;

/// `cladding.rs` `mod wall`: the drawn wall frames, in sheet order, as [`presets::minimal_d4`]
/// roles — corner, top, column head (strip end), column body (strip), interior, inside, double
/// inside — which is the order `two_surface` expects them in.
const WALL_FRAMES: usize = 7;

struct Import {
    root: PathBuf,
    p: ProjectDir,
    /// Each sheet as the game sees it, for the round-trip check.
    sheets: Vec<(String, RgbaImage)>,
    notes: Vec<String>,
}

fn main() -> Result<()> {
    let usage = "usage: run2d-import <run-2d checkout> -o <project folder> [--showcase]";
    let mut args = std::env::args().skip(1);
    let mut root = None;
    let mut out = None;
    let mut showcase = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" | "--out" => out = Some(PathBuf::from(args.next().context("-o needs a folder")?)),
            "--showcase" => showcase = true,
            "-h" | "--help" => {
                println!("{usage}");
                return Ok(());
            }
            _ => root = Some(PathBuf::from(a)),
        }
    }
    let (root, out) = (root.context(usage)?, out.context(usage)?);
    ensure!(root.join("assets/images/player.png").exists(), "{} does not look like a run-2d checkout with its sheets", root.display());

    let mut import = Import { root, p: ProjectDir::new("run-2d"), sheets: Vec::new(), notes: Vec::new() };
    import.p.grid = TILE;
    import.palette()?;
    import.sheets()?;
    if showcase {
        import.showcase()?;
    }
    import.p.palette.drop_empty_sections();
    import.verify()?;
    import.colours_in_code()?;

    let problems = import.p.validate();
    ensure!(problems.is_empty(), "the generated project has problems:\n{}", problems.join("\n"));
    clear_stale(&out, &import.p)?;
    let written = import.p.save(&out)?;

    for note in &import.notes {
        println!("note: {note}");
    }
    println!(
        "{}: wrote {} files; {} palette entries in {} sections, {} sprites, {} terrains",
        out.display(),
        written.len(),
        import.p.palette.len(),
        import.p.palette.sections().len(),
        import.p.sprites.len(),
        import.p.terrains.len()
    );
    Ok(())
}

/// Remove `.pxl` files a previous run wrote that this one does not, so the folder is exactly the
/// project.
fn clear_stale(out: &Path, p: &ProjectDir) -> Result<()> {
    let keep: std::collections::BTreeSet<PathBuf> = p.to_files().keys().map(|k| out.join(k)).collect();
    let mut dirs = vec![out.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries {
            let path = entry?.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "pxl") && !keep.contains(&path) {
                std::fs::remove_file(&path)?;
                println!("removed {}", path.display());
            }
        }
    }
    Ok(())
}

impl Import {
    fn palette(&mut self) -> Result<()> {
        let maps = self.map_backgrounds()?;
        let pal = &mut self.p.palette;
        pal.begin_section("DawnBringer 32");
        for (name, v) in DB32 {
            pal.push(name, hex(v))?;
        }
        pal.begin_section("Player keys");
        for (name, v) in PLAYER_KEYS {
            pal.push(name, hex(v))?;
        }
        pal.begin_section("Clothing");
        for (name, [r, g, b]) in CLOTHING {
            pal.push(name, srgb(r, g, b))?;
        }
        pal.begin_section("Skin tones");
        for (i, [r, g, b]) in SKIN_TONES.iter().enumerate() {
            let name = if i == 0 { "sheet".to_string() } else { format!("tone {i}") };
            pal.push(name, srgb(*r, *g, *b))?;
        }
        pal.begin_section("World");
        pal.push("sky", hex(SKY))?;
        pal.push("terrain fill", hex(TERRAIN_FILL))?;
        pal.push("bullet", srgb(BULLET[0], BULLET[1], BULLET[2]))?;
        pal.push("spike", srgb(SPIKE[0], SPIKE[1], SPIKE[2]))?;
        pal.push("blast", [255; 4])?;
        pal.begin_section("Net");
        let mesh = hex(MESH);
        pal.push("strand", [mesh[0], mesh[1], mesh[2], (STRAND_ALPHA * 255.0).round() as u8])?;
        pal.push("hole", [mesh[0], mesh[1], mesh[2], (HOLE_ALPHA * 255.0).round() as u8])?;
        pal.begin_section("Weapon markers");
        for (name, [r, g, b]) in WEAPON_MARKERS {
            pal.push(name, srgb(r, g, b))?;
        }
        pal.begin_section("UI");
        for (name, [r, g, b, a]) in UI {
            pal.push(name, srgba(r, g, b, a))?;
        }
        pal.begin_section("Editor");
        for (name, [r, g, b, a]) in EDITOR {
            pal.push(name, srgba(r, g, b, a))?;
        }
        pal.begin_section("Diagram");
        for (name, [r, g, b]) in MATERIALS {
            pal.push(name, srgb(r, g, b))?;
        }
        pal.push("custom level", srgb(CUSTOM_LEVEL[0], CUSTOM_LEVEL[1], CUSTOM_LEVEL[2]))?;
        for (name, [r, g, b]) in maps {
            pal.push(format!("map {name}"), srgb(r, g, b))?;
        }
        // Anything the sheets use that nothing above names lands here, as found.
        pal.begin_section("Imported");
        Ok(())
    }

    /// `arena/maps.rs`: each built-in map's name and its picker colour, in file order.
    fn map_backgrounds(&self) -> Result<Vec<(String, [f32; 3])>> {
        let text = std::fs::read_to_string(self.root.join("src/arena/maps.rs"))?;
        let mut out = Vec::new();
        let mut name = None;
        for line in text.lines().map(str::trim) {
            if let Some(rest) = line.strip_prefix("name: \"") {
                name = rest.split('"').next().map(str::to_string);
            } else if let Some(rest) = line.strip_prefix("background: Color::srgb(") {
                let v: Vec<f32> = rest
                    .trim_end_matches("),")
                    .split(',')
                    .map(|s| s.trim().parse())
                    .collect::<Result<_, _>>()
                    .with_context(|| format!("reading {line:?}"))?;
                ensure!(v.len() == 3, "reading {line:?}");
                out.push((name.take().unwrap_or_else(|| format!("#{}", out.len())), [v[0], v[1], v[2]]));
            }
        }
        ensure!(!out.is_empty(), "found no map backgrounds in maps.rs");
        Ok(out)
    }

    /// The palette index for a colour, preferring `sections` in order, then DB32, then anything;
    /// a colour nothing holds yet is added to the last section and noted.
    fn index(&mut self, rgba: [u8; 4], sections: &[&str], what: &str) -> Result<u8> {
        if rgba[3] == 0 {
            return Ok(0);
        }
        let pal = &mut self.p.palette;
        for s in sections.iter().chain(&["DawnBringer 32"]) {
            if let Some(i) = pal.find(rgba, Some(s)) {
                return Ok(i);
            }
        }
        if let Some(i) = pal.find(rgba, None) {
            return Ok(i);
        }
        let name = format!("{what} #{:02x}{:02x}{:02x}{:02x}", rgba[0], rgba[1], rgba[2], rgba[3]);
        self.notes.push(format!("added {name}, which no named colour matches"));
        Ok(pal.push(name, rgba)?)
    }

    fn indexed(&mut self, img: &RgbaImage, sections: &[&str], what: &str) -> Result<Indexed> {
        let mut out = Indexed::new(img.w, img.h);
        for (i, c) in img.px.chunks_exact(4).enumerate() {
            out.px[i] = self.index([c[0], c[1], c[2], c[3]], sections, what)?;
        }
        Ok(out)
    }

    fn named(&self, section: &str, name: &str) -> u8 {
        self.p.palette.named(section, name).unwrap_or_else(|| panic!("no {name:?} in {section:?}"))
    }

    fn sheets(&mut self) -> Result<()> {
        self.sheet("player", &["Player keys"], &POSES)?;
        self.sheet("weapons", &[], &WEAPONS.map(|w| w.0))?;
        self.sheet("props", &[], &PROPS)?;
        self.sheet("emotes", &[], &[])?;
        self.sheet("platforms", &[], &PLATFORMS)?;
        self.walls()?;
        self.animations();

        // Drawn, never exported: kept where it can be finished.
        let path = self.root.join("assets/images/walls2.aseprite");
        if path.exists() {
            let ase = aseprite::parse(&std::fs::read(&path)?)?;
            let mut s = SpriteFile::new(ase.width, ase.height);
            s.frames.clear();
            for f in 0..ase.frames.len() {
                let pic = self.indexed(&rgba(&ase.visible(f), ase.width, ase.height), &[], "walls2")?;
                let mut frame = SpriteFrame::new(ase.width, ase.height);
                frame.set_layer("art", pic);
                s.frames.push(frame);
            }
            self.p.sprites.insert("wip/walls2".into(), s);
            self.notes.push(format!("walls2.aseprite has no PNG and nothing uses it; its {} frames are wip/walls2", ase.frames.len()));
        }
        Ok(())
    }

    /// The cycles the game plays out of its sheets in code, as animations of them, so they can be
    /// watched in the editor: `player.rs` `animate` and `weapon.rs` `two_frame`.
    fn animations(&mut self) {
        let step = |frame: usize, duration: f32| Step { frame, duration };
        let named = |s: &SpriteFile, name: &str| s.frames.iter().position(|f| f.name.as_deref() == Some(name));
        if let Some(player) = self.p.sprites.get_mut("player") {
            for (anim, [a, b]) in [("run", ["run_1", "run_2"]), ("crouch_walk", ["crouch", "crouch_walk"])] {
                if let (Some(a), Some(b)) = (named(player, a), named(player, b)) {
                    player.anims.insert(anim.into(), vec![step(a, RUN_FRAME_TIME), step(b, RUN_FRAME_TIME)]);
                }
            }
        }
        if let Some(weapons) = self.p.sprites.get_mut("weapons") {
            let (ring, core) = (SHELL_RING_TICKS * TICK, SHELL_CORE_TICKS * TICK);
            for (anim, [a, b]) in [("shell", ["shell_1", "shell_2"]), ("rocket", ["rocket_1", "rocket_2"])] {
                if let (Some(a), Some(b)) = (named(weapons, a), named(weapons, b)) {
                    weapons.anims.insert(anim.into(), vec![step(a, ring), step(b, core)]);
                }
            }
        }
    }

    /// A sheet's PNG, as the game sees it.
    fn png(&self, name: &str) -> Result<RgbaImage> {
        let path = self.root.join(format!("assets/images/{name}.png"));
        let img = png_io::decode(&std::fs::read(&path)?).with_context(|| format!("decoding {}", path.display()))?.to_rgba();
        ensure!(img.h == TILE && img.w % TILE == 0, "{name}.png is {}x{}, not a row of {TILE}px frames", img.w, img.h);
        Ok(img)
    }

    /// One sheet as a sprite: the PNG on `art`, hidden `.aseprite` layers on layers of their own.
    fn sheet(&mut self, name: &str, sections: &[&str], frame_names: &[&str]) -> Result<()> {
        let img = self.png(name)?;
        let count = (img.w / TILE) as usize;
        let pic = self.indexed(&img, sections, name)?;
        let mut s = SpriteFile::new(TILE, TILE);
        s.frames.clear();
        if !frame_names.is_empty() && frame_names.len() != count {
            self.notes.push(format!("{name}.png has {count} frames, but {} names are known for it; left unnamed", frame_names.len()));
        }
        for f in 0..count {
            let mut frame = SpriteFrame::new(TILE, TILE);
            if frame_names.len() == count {
                frame.name = Some(frame_names[f].to_string());
            }
            frame.set_layer("art", pic.crop(f as u32 * TILE, 0, TILE, TILE));
            s.frames.push(frame);
        }
        self.sheets.push((name.to_string(), img.clone()));

        if let Some(ase) = self.aseprite(name, count, &img)? {
            for (l, layer) in ase.layers.iter().enumerate().filter(|(_, l)| !l.visible && !l.group) {
                let layer_name = pixelle_core::import::snake(&layer.name);
                s.layers.push(SpriteLayer { name: layer_name.clone(), export: false, visible: false });
                for f in 0..count {
                    let pic = self.indexed(&rgba(&ase.frames[f][l], ase.width, ase.height), sections, &format!("{name} {}", layer.name))?;
                    s.frames[f].set_layer(&layer_name, pic);
                }
                self.notes.push(format!("{name}.aseprite's hidden layer {:?} is the layer {layer_name:?}, hidden and not exported", layer.name));
            }
        }
        self.p.sprites.insert(name.into(), s);
        Ok(())
    }

    /// A sheet's `.aseprite`, checked against its PNG.
    fn aseprite(&mut self, name: &str, count: usize, png: &RgbaImage) -> Result<Option<aseprite::Ase>> {
        let path = self.root.join(format!("assets/images/{name}.aseprite"));
        if !path.exists() {
            return Ok(None);
        }
        let ase = aseprite::parse(&std::fs::read(&path)?).with_context(|| format!("reading {}", path.display()))?;
        if ase.frames.len() != count || (ase.width, ase.height) != (TILE, TILE) {
            self.notes.push(format!("{name}.aseprite has {} frames of {}x{}, the PNG {count}; its hidden layers were left out", ase.frames.len(), ase.width, ase.height));
            return Ok(None);
        }
        let stale: Vec<usize> = (0..count)
            .filter(|&f| normalise(&rgba(&ase.visible(f), ase.width, ase.height)) != normalise(&crop(png, IRect::new((f as u32 * TILE) as i32, 0, TILE, TILE))))
            .collect();
        if !stale.is_empty() {
            self.notes.push(format!("{name}.png differs from {name}.aseprite in frames {stale:?}; the PNG (what the game draws) was used"));
        }
        Ok(Some(ase))
    }

    /// The walls: `cladding.rs`'s seven frames, each turned every way the terrain draws it.
    fn walls(&mut self) -> Result<()> {
        let img = self.png("walls")?;
        ensure!(img.w / TILE == WALL_FRAMES as u32, "walls.png has {} frames, cladding.rs draws from {WALL_FRAMES}", img.w / TILE);
        let pic = self.indexed(&img, &[], "walls")?;
        let canonical: Vec<Indexed> = (0..WALL_FRAMES as u32).map(|f| pic.crop(f * TILE, 0, TILE, TILE)).collect();
        let preset = Preset::TwoSurface { mirror: false };
        let mut t = TerrainFile::new(preset, TILE);
        t.tiles = presets::expand_mirrored(&preset.two_surface().slots, &canonical);
        self.p.terrains.insert("walls".into(), t);
        self.sheets.push(("walls".into(), img.clone()));
        if let Some(ase) = self.aseprite("walls", WALL_FRAMES, &img)?
            && ase.layers.iter().any(|l| !l.visible && !l.group)
        {
            self.notes.push("walls.aseprite has hidden layers; a terrain has one layer, so they were left out".into());
        }
        Ok(())
    }

    /// What the showcase shows off: anchors, cuts, recolour slots and the art drawn in code.
    fn showcase(&mut self) -> Result<()> {
        let player = self.p.sprites.get_mut("player").context("player")?;
        for (i, f) in player.frames.iter_mut().enumerate() {
            f.points.insert("hand".into(), hand_pixel(i));
        }
        let idle = &mut player.frames[0];
        for (name, [l, t, r, b]) in RAGDOLL_CUTS {
            idle.rects.insert(name.into(), IRect::new(l, t, (r - l) as u32, (b - t) as u32));
        }
        // `killfeed.rs` `FACE_CUT`.
        idle.rects.insert("face".into(), IRect::new(2, 0, 4, 4));

        let weapons = self.p.sprites.get_mut("weapons").context("weapons")?;
        for (f, (_, held, shoots, [dx, dy])) in weapons.frames.iter_mut().zip(WEAPONS) {
            if held {
                f.points.insert("trigger".into(), TRIGGER);
            }
            if shoots {
                f.points.insert("muzzle".into(), MUZZLE);
            }
            f.points.insert("rest".into(), IVec2::new(4 + dx, 4 - dy));
        }

        // The bullet (`weapon.rs`): a 3x1 quad.
        let bullet = self.named("World", "bullet");
        let mut s = SpriteFile::new(3, 1);
        s.frames[0].set_layer("art", Indexed { w: 3, h: 1, px: vec![bullet; 3] });
        self.p.sprites.insert("fx/bullet".into(), s);

        // The blast (`weapon.rs` `blast_radius`): a white disc, a frame per tick.
        let blast = self.named("World", "blast");
        let c = BLAST as f32 / 2.0;
        let mut s = SpriteFile::new(BLAST, BLAST);
        s.frames.clear();
        for f in 0..EXPLOSION_TICKS {
            let r = blast_radius(f);
            let mut pic = Indexed::new(BLAST, BLAST);
            for y in 0..BLAST {
                for x in 0..BLAST {
                    let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
                    // Strictly inside: a disc of radius 0 draws nothing, as the scaled mesh does.
                    if dx * dx + dy * dy < r * r {
                        pic.set(x, y, blast);
                    }
                }
            }
            let mut frame = SpriteFrame::new(BLAST, BLAST);
            frame.duration = TICK;
            frame.set_layer("art", pic);
            s.frames.push(frame);
        }
        s.pivot = Some(IVec2::splat(BLAST as i32 / 2));
        s.tags.insert("radius".into(), "18 * sin(pi * tick / 8)".into());
        self.p.sprites.insert("fx/explosion".into(), s);

        // Recolour slots: the sheet's tones, and where a player picks replacements from.
        for (name, _) in PLAYER_KEYS {
            let options = if name == "skin" { "Skin tones" } else { "Clothing" };
            let index = self.named("Player keys", name);
            self.p.swaps.push(SwapSlot { name: name.into(), index, options: options.into() });
        }
        Ok(())
    }

    /// Every sheet comes back out of the project exactly, and the walls terrain draws what
    /// `cladding.rs` would.
    fn verify(&mut self) -> Result<()> {
        let mut notes = Vec::new();
        let baked = bake::bake_folder(&self.p);
        for (name, png) in &self.sheets {
            if name == "walls" {
                continue;
            }
            let back = baked.sheet_rgba(name).with_context(|| format!("no sprite {name}"))?;
            ensure!(normalise(&back) == normalise(png), "{name}.png does not survive the round trip");
        }

        // `derive_wall_sheet`: the drawn frames, then TOP, COLUMN_HEAD and COLUMN_BODY transposed.
        let (_, walls) = self.sheets.iter().find(|(n, _)| n == "walls").context("walls")?;
        let derived: Vec<RgbaImage> = (0..10)
            .map(|f| {
                let src = if f < 7 { f } else { [1, 2, 3][f - 7] };
                let img = crop(walls, IRect::new(src as i32 * TILE as i32, 0, TILE, TILE));
                if f >= 7 { transpose(&img) } else { img }
            })
            .collect();
        let terrain = baked.terrain("walls").context("walls terrain")?;
        let atlas = Indexed { w: baked.atlas_width, h: baked.atlas_height, px: baked.atlas.clone() };
        let mut quartered = Vec::new();
        for mask in 0..=255u8 {
            let solid = |d: IVec2| {
                let slot = pixelle_core::tiles::NEIGHBOURS.iter().position(|n| *n == d);
                slot.is_none_or(|s| mask & (1 << s) != 0)
            };
            let mut ours = Indexed::new(TILE, TILE);
            for piece in terrain.cell(solid) {
                let src = atlas.crop(piece.source.x as u32, piece.source.y as u32, piece.source.w, piece.source.h);
                ours.blit(&src.transformed(pixelle_core::D4::new(false, piece.flip_x, piece.flip_y)), piece.at.min());
            }
            let ours = ours.to_rgba(&self.p.palette);
            // y up, as run-2d asks: open(dc, dr).
            let open = |dc: i32, dr: i32| !solid(IVec2::new(dc, -dr));
            let (theirs, whole) = run2d_cell(&derived, open);
            if normalise(&ours) == normalise(&theirs) {
                continue;
            }
            // Where cladding.rs draws a whole frame, the terrain must draw exactly that. Where it
            // builds a cell from quarters, it mirrors each frame's top-left quarter into every
            // corner; with the mirror off a quarter comes from that corner of the tile as drawn,
            // so a frame that is not symmetric differs there by its own texture.
            ensure!(!whole, "walls terrain differs from cladding.rs for neighbourhood {mask:08b}, a whole frame");
            quartered.push(normalise(&ours).iter().zip(normalise(&theirs)).filter(|(a, b)| *a != b).count());
        }

        if !quartered.is_empty() {
            notes.push(format!(
                "walls: {} of 256 neighbourhoods built from quarters differ from cladding.rs by at most {} pixels, because \
                 quarters now come from each tile's own corners instead of a mirrored top-left quarter",
                quartered.len(),
                quartered.iter().max().unwrap()
            ));
        }

        // Explosion frames are exactly as wide as the disc the game scales.
        if let Some(blast) = self.p.sprites.get("fx/explosion") {
            for f in 0..EXPLOSION_TICKS as usize {
                let pic = blast.composite(f);
                let across = (0..BLAST).filter(|&x| pic.get(x, BLAST / 2) != 0).count() as f32;
                let r = blast_radius(f as u32);
                ensure!((across - 2.0 * r).abs() <= 1.5, "blast frame {f}: {across}px across for radius {r}");
            }
        }
        self.notes.extend(notes);
        Ok(())
    }
}

impl Import {
    /// Every colour the game's code names should be in the palette, so the art and the code
    /// cannot drift apart unseen. Debug overlays are left out. Anything missing is reported.
    fn colours_in_code(&mut self) -> Result<()> {
        // Debug drawing only (`weapon.rs` melee gizmos), never part of the game's look.
        const DEBUG: [&str; 4] = ["SWUNG", "TARGET_BOX", "NETTED_BOX", "PIECE_BOX"];
        let mut files = Vec::new();
        let mut dirs = vec![self.root.join("src")];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir)? {
                let path = entry?.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    files.push(path);
                }
            }
        }
        files.sort();
        for file in files {
            let name = file.strip_prefix(&self.root).unwrap_or(&file).display().to_string();
            if name.ends_with("_tests.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&file)?;
            let text = text.split("#[cfg(test)]").next().unwrap_or("");
            for (n, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                if DEBUG.iter().any(|d| code.contains(d)) {
                    continue;
                }
                for c in literal_colours(code) {
                    if self.p.palette.find(c, None).is_none() {
                        self.notes.push(format!(
                            "{name}:{}: #{:02x}{:02x}{:02x}{:02x} is not in the palette: {}",
                            n + 1,
                            c[0],
                            c[1],
                            c[2],
                            c[3],
                            line.trim()
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

/// `Color::srgb(..)`, `Color::srgba(..)` and `Color::srgb_u8(..)` literals on a line, converted
/// as the game converts them.
fn literal_colours(line: &str) -> Vec<[u8; 4]> {
    let mut out = Vec::new();
    for (prefix, bytes) in [("Color::srgb_u8(", true), ("Color::srgba(", false), ("Color::srgb(", false)] {
        let mut rest = line;
        while let Some(at) = rest.find(prefix) {
            rest = &rest[at + prefix.len()..];
            let Some(end) = rest.find(')') else { break };
            let args: Vec<&str> = rest[..end].split(',').map(str::trim).filter(|a| !a.is_empty()).collect();
            let parsed: Option<Vec<f32>> = args
                .iter()
                .map(|a| {
                    if bytes {
                        u8::from_str_radix(a.trim_start_matches("0x"), if a.starts_with("0x") { 16 } else { 10 }).ok().map(|v| v as f32 / 255.0)
                    } else {
                        a.parse::<f32>().ok()
                    }
                })
                .collect();
            match parsed.as_deref() {
                Some([r, g, b]) => out.push(srgba(*r, *g, *b, 1.0)),
                Some([r, g, b, a]) => out.push(srgba(*r, *g, *b, *a)),
                _ => {}
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// `weapon.rs:2558`.
fn blast_radius(age: u32) -> f32 {
    if age >= EXPLOSION_TICKS {
        return 0.0;
    }
    EXPLOSION_RADIUS * (age as f32 / EXPLOSION_TICKS as f32 * std::f32::consts::PI).sin()
}

fn rgba(px: &[[u8; 4]], w: u32, h: u32) -> RgbaImage {
    RgbaImage {
        w,
        h,
        px: px.iter().flatten().copied().collect(),
    }
}

fn crop(img: &RgbaImage, r: IRect) -> RgbaImage {
    let mut out = RgbaImage::new(r.w, r.h);
    for y in 0..r.h {
        for x in 0..r.w {
            let s = (((r.y as u32 + y) * img.w + r.x as u32 + x) * 4) as usize;
            let d = ((y * r.w + x) * 4) as usize;
            out.px[d..d + 4].copy_from_slice(&img.px[s..s + 4]);
        }
    }
    out
}

fn transpose(img: &RgbaImage) -> RgbaImage {
    let mut out = RgbaImage::new(img.h, img.w);
    for y in 0..img.h {
        for x in 0..img.w {
            let s = ((y * img.w + x) * 4) as usize;
            let d = ((x * out.w + y) * 4) as usize;
            out.px[d..d + 4].copy_from_slice(&img.px[s..s + 4]);
        }
    }
    out
}

/// Fully transparent pixels compare equal whatever colour they carry.
fn normalise(img: &RgbaImage) -> Vec<[u8; 4]> {
    img.px
        .chunks_exact(4)
        .map(|c| if c[3] == 0 { [0; 4] } else { [c[0], c[1], c[2], c[3]] })
        .collect()
}

/// `cladding.rs` at e59b281: `Around`, `whole_frame`, `quarter` and the drawing in `clad_terrain`,
/// ported to produce the picture of one cell, and whether it is a whole frame. `open(dc, dr)` is
/// whether the neighbour that many columns across and rows up is open.
fn run2d_cell(sheet: &[RgbaImage], open: impl Fn(i32, i32) -> bool) -> (RgbaImage, bool) {
    const CORNER: usize = 0;
    const TOP: usize = 1;
    const COLUMN_HEAD: usize = 2;
    const COLUMN_BODY: usize = 3;
    const INTERIOR: usize = 4;
    const INSIDE: usize = 5;
    const DOUBLE_INSIDE: usize = 6;
    const LEFT: usize = 7;
    const BAR_END: usize = 8;
    const BAR_BODY: usize = 9;
    let (n, s, e, w) = (open(0, 1), open(0, -1), open(1, 0), open(-1, 0));
    let (ne, nw, se, sw) = (open(1, 1), open(-1, 1), open(1, -1), open(-1, -1));
    let whole = match (n, s, w, e) {
        (false, false, false, false) => match (ne, nw, se, sw) {
            (false, false, false, false) => Some((INTERIOR, false, false)),
            (true, false, false, false) => Some((INSIDE, false, false)),
            (false, true, false, false) => Some((INSIDE, true, false)),
            (false, false, true, false) => Some((INSIDE, false, true)),
            (false, false, false, true) => Some((INSIDE, true, true)),
            (true, true, false, false) => Some((DOUBLE_INSIDE, false, false)),
            (false, false, true, true) => Some((DOUBLE_INSIDE, false, true)),
            _ => None,
        },
        (true, false, false, false) if !se && !sw => Some((TOP, false, false)),
        (false, true, false, false) if !ne && !nw => Some((TOP, false, true)),
        (false, false, true, false) if !ne && !se => Some((LEFT, false, false)),
        (false, false, false, true) if !nw && !sw => Some((LEFT, true, false)),
        (true, false, true, false) if !se => Some((CORNER, false, false)),
        (true, false, false, true) if !sw => Some((CORNER, true, false)),
        (false, true, true, false) if !ne => Some((CORNER, false, true)),
        (false, true, false, true) if !nw => Some((CORNER, true, true)),
        (false, false, true, true) => Some((COLUMN_BODY, false, false)),
        (true, true, false, false) => Some((BAR_BODY, false, false)),
        (true, false, true, true) => Some((COLUMN_HEAD, false, false)),
        (false, true, true, true) => Some((COLUMN_HEAD, false, true)),
        (true, true, true, false) => Some((BAR_END, false, false)),
        (true, true, false, true) => Some((BAR_END, true, false)),
        _ => None,
    };
    if let Some((f, fx, fy)) = whole {
        return (flip(&sheet[f], fx, fy), true);
    }
    let quarter = |vertical: bool, horizontal: bool, diagonal: bool| match (vertical, horizontal, diagonal) {
        (true, true, _) => (CORNER, false),
        (true, false, _) => (TOP, false),
        (false, true, _) => (LEFT, false),
        (false, false, true) => (INSIDE, true),
        (false, false, false) => (INTERIOR, false),
    };
    let half = TILE as u32 / 2;
    let mut out = RgbaImage::new(TILE as u32, TILE as u32);
    for (vertical, horizontal, diagonal, right, bottom) in
        [(n, w, nw, false, false), (n, e, ne, true, false), (s, w, sw, false, true), (s, e, se, true, true)]
    {
        let (f, mirrored) = quarter(vertical, horizontal, diagonal);
        let from = if mirrored { half } else { 0 };
        let piece = flip(&crop(&sheet[f], IRect::new(from as i32, 0, half, half)), right != mirrored, bottom);
        let (ox, oy) = (if right { half } else { 0 }, if bottom { half } else { 0 });
        for y in 0..half {
            for x in 0..half {
                let s = ((y * half + x) * 4) as usize;
                let d = (((oy + y) * TILE as u32 + ox + x) * 4) as usize;
                out.px[d..d + 4].copy_from_slice(&piece.px[s..s + 4]);
            }
        }
    }
    (out, false)
}

fn flip(img: &RgbaImage, fx: bool, fy: bool) -> RgbaImage {
    let mut out = RgbaImage::new(img.w, img.h);
    for y in 0..img.h {
        for x in 0..img.w {
            let (sx, sy) = (if fx { img.w - 1 - x } else { x }, if fy { img.h - 1 - y } else { y });
            let s = ((sy * img.w + sx) * 4) as usize;
            let d = ((y * img.w + x) * 4) as usize;
            out.px[d..d + 4].copy_from_slice(&img.px[s..s + 4]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_literals_are_found_and_converted_like_the_game() {
        let line = "a: Color::srgb(0.30, 0.33, 0.40), b: Color::srgba(1.0, 1.0, 1.0, 0.85) Color::srgb_u8(0x4b, 0x94, 0xba)";
        let got = literal_colours(line);
        assert!(got.contains(&[77, 84, 102, 255]), "0.30 rounds up, as f32 * 255 does in the game");
        assert!(got.contains(&[255, 255, 255, 217]));
        assert!(got.contains(&[0x4b, 0x94, 0xba, 255]));
        assert_eq!(got.len(), 3);
        assert!(literal_colours("Color::srgb(SKY)").is_empty());
    }
}
