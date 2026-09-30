//! Generate `run2d.pxl`: every piece of run-2d's art on one canvas.
//!
//! ```text
//! cargo run -p run2d-import -- /path/to/run-2d [-o run2d.pxl]
//! ```
//!
//! What goes in:
//! - the six sheets, from their PNGs (what the game draws today), with any hidden `.aseprite`
//!   layers kept beside them as sketches, and the unexported `walls2.aseprite` as work in progress;
//! - every colour the game's code names, as labelled palette sections;
//! - the art the game draws in code — the bullet, the blast, the tinted flag pads, the net, the
//!   transposed wall frames, the editor's cursors — as pixels an artist can now paint over;
//! - names for every frame, the anchor pixels the code seats weapons by, and the recolour slots;
//! - the wall and platform autotiling as terrains, and a showcase level built from all of it.
//!
//! It is deterministic and re-runnable: until the game reads the file, regenerate it whenever the
//! sheets change. Before writing, it checks that every sheet comes back out of the file pixel for
//! pixel, and that the wall terrain reproduces the sheet `cladding.rs` derives at run time.

mod aseprite;
mod colors;

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};
use pixelle_core::layer::{Instance, LayerKind, TerrainLayer};
use pixelle_core::render::{Renderer, RgbaImage};
use pixelle_core::tiles::presets::{self, MinimalD4};
use pixelle_core::{
    Bookmark, D4, Frame, Grid, IRect, IVec2, LayerId, Project, Sprite, SwapSlot, Terrain, Tileset,
    format, png_io,
};

use colors::*;

/// Every frame in run-2d is this big (`atlas::FRAME`), and so is a level cell (`arena::CELL`).
const TILE: i32 = 8;
/// Sheets start this far right, leaving room for an animation's preview to play beside it.
const LEFT: i32 = 16;
const PLAYER_Y: i32 = 0;
const WEAPONS_Y: i32 = 16;
const PROPS_Y: i32 = 32;
const EMOTES_Y: i32 = 48;
const WALLS_Y: i32 = 64;
const PLATFORMS_Y: i32 = 80;
const BULLET_Y: i32 = 96;
const FX_Y: i32 = 112;
/// The blast's frames: wide enough for its widest, 18px radius, with room for its preview.
const BLAST: u32 = 37;
const BLAST_X: i32 = 56;
const BLAST_STRIDE: i32 = 40;
/// The top of the showcase level.
const MOCK_Y: i32 = 176;

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
const IDLE: u32 = 0;
const RUN_1: u32 = 1;
const SLIDE: u32 = 3;
const JUMP: u32 = 4;
const JUMP_ARMED: u32 = 5;
const WALL_SLIDE_ARMED: u32 = 7;
const CROUCH: u32 = 8;

/// `weapon.rs:827` `hand_pixel`: where each pose holds a weapon.
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

/// `weapon.rs:507-509`.
const TRIGGER: IVec2 = IVec2::new(4, 4);
const MUZZLE: IVec2 = IVec2::new(6, 3);
/// `player.rs:55`: seconds between the frames of the run and crouch-walk cycles.
const RUN_FRAME_TIME: f32 = 0.11;
/// `weapon.rs:349-351`: a shell (and a rocket) shows its first frame this many ticks, then its
/// second for the other.
const SHELL_RING_TICKS: f32 = 8.0;
const SHELL_CORE_TICKS: f32 = 4.0;
/// `weapon.rs:457-458`.
const EXPLOSION_TICKS: u32 = 8;
const EXPLOSION_RADIUS: f32 = 18.0;

/// `weapons.png`, in sheet order: name, whether it is held (seated by `trigger`), whether it
/// shoots (has a `muzzle`), and `weapon.rs:733` `ART_OFFSET` (y up) for when it lies loose.
const WEAPONS: [(&str, bool, bool, [i32; 2]); 15] = [
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
];

/// `cladding.rs:182-218`: the drawn wall frames, then the three `derive_wall_sheet` transposes.
const WALL_FRAMES: [&str; 10] = [
    "corner",
    "top",
    "column_head",
    "column_body",
    "interior",
    "inside",
    "double_inside",
    "left",
    "bar_end",
    "bar_body",
];

/// The showcase: `#` earth, `=` steel girder, 40 cells by 16, from the top-left of the level.
const LEVEL: [&str; 16] = [
    "........................................",
    "........................................",
    "....====..........................====..",
    "........................................",
    "...........................#............",
    "..........#####............#............",
    "...........................#.....###....",
    "...............====..............###....",
    "#...............................=====..#",
    "#......................................#",
    "#.....##.......................#......##",
    "#.....##.........===...........#....####",
    "#.....##.......................#....####",
    "########################################",
    "########################################",
    "########################################",
];

struct Layers {
    sky: LayerId,
    art: LayerId,
    net: LayerId,
    sketch: LayerId,
    wip: LayerId,
    level: LayerId,
    props: LayerId,
    actors: LayerId,
}

struct Import {
    root: PathBuf,
    p: Project,
    l: Layers,
    /// Each sheet's own pixels as the game sees them, for the round-trip check.
    sheets: Vec<(String, IVec2, RgbaImage)>,
    notes: Vec<String>,
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut root = None;
    let mut out = PathBuf::from("run2d.pxl");
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" | "--out" => out = args.next().context("-o needs a path")?.into(),
            "-h" | "--help" => {
                println!("usage: run2d-import <run-2d checkout> [-o run2d.pxl]");
                return Ok(());
            }
            _ => root = Some(PathBuf::from(a)),
        }
    }
    let root = root.context("usage: run2d-import <run-2d checkout> [-o run2d.pxl]")?;
    ensure!(
        root.join("assets/images/player.png").exists(),
        "{} does not look like a run-2d checkout",
        root.display()
    );

    let mut import = Import::new(root)?;
    import.sheets()?;
    import.code_art()?;
    import.sprites()?;
    import.terrains();
    import.level()?;
    import.p.palette.drop_empty_sections();
    import.verify()?;

    let problems = import.p.validate();
    ensure!(problems.is_empty(), "the generated project has problems:\n{}", problems.join("\n"));
    format::save_file(&import.p, &out)?;

    for note in &import.notes {
        println!("note: {note}");
    }
    let bytes = std::fs::metadata(&out)?.len();
    println!(
        "wrote {} ({bytes} bytes): {} palette entries in {} sections, {} layers, {} sprites, {} terrains",
        out.display(),
        import.p.palette.len(),
        import.p.palette.sections().len(),
        import.p.layers.len(),
        import.p.sprites.len(),
        import.p.terrains.len()
    );
    Ok(())
}

fn strip(y: i32, i: i32) -> IRect {
    IRect::new(LEFT + i * TILE, y, TILE as u32, TILE as u32)
}

impl Import {
    fn new(root: PathBuf) -> Result<Self> {
        let mut p = Project::new("run-2d");
        p.grid = TILE as u32;
        let px = || LayerKind::Pixels(Grid::new());
        let l = Layers {
            sky: p.add_layer("Sky", px()),
            art: p.add_layer("Art", px()),
            net: p.add_layer("Net", px()),
            sketch: p.add_layer("Sketch", px()),
            wip: p.add_layer("WIP", px()),
            level: p.add_layer(
                "Level",
                LayerKind::Terrain(TerrainLayer {
                    cell_size: TILE as u32,
                    cells: Grid::new(),
                }),
            ),
            props: p.add_layer("Props", LayerKind::Instances(Vec::new())),
            actors: p.add_layer("Actors", LayerKind::Instances(Vec::new())),
        };
        // Everything but the art and the net is for looking at, not for export.
        for id in [l.sky, l.sketch, l.wip, l.level, l.props, l.actors] {
            p.layer_mut(id).unwrap().reference = true;
        }
        p.layer_mut(l.sketch).unwrap().visible = false;

        let mut import = Import {
            root,
            p,
            l,
            sheets: Vec::new(),
            notes: Vec::new(),
        };
        import.palette()?;
        Ok(import)
    }

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

    fn paint(&mut self, layer: LayerId, at: IVec2, img: &RgbaImage, sections: &[&str], what: &str) -> Result<()> {
        for y in 0..img.h {
            for x in 0..img.w {
                let i = ((y * img.w + x) * 4) as usize;
                let c: [u8; 4] = img.px[i..i + 4].try_into()?;
                let v = self.index(c, sections, what)?;
                if v != 0 {
                    self.grid(layer).set(at + IVec2::new(x as i32, y as i32), v);
                }
            }
        }
        Ok(())
    }

    fn grid(&mut self, layer: LayerId) -> &mut Grid {
        self.p.layer_mut(layer).and_then(|l| l.pixels_mut()).expect("a pixel layer")
    }

    fn sheets(&mut self) -> Result<()> {
        self.sheet("player", PLAYER_Y, 10, &["Player keys"])?;
        self.sheet("weapons", WEAPONS_Y, 15, &[])?;
        self.sheet("props", PROPS_Y, 4, &[])?;
        self.sheet("emotes", EMOTES_Y, 4, &[])?;
        self.sheet("walls", WALLS_Y, 7, &[])?;
        self.sheet("platforms", PLATFORMS_Y, 3, &[])?;

        // Drawn, never exported: kept where it can be finished.
        let path = self.root.join("assets/images/walls2.aseprite");
        if path.exists() {
            let ase = aseprite::parse(&std::fs::read(&path)?)?;
            for f in 0..ase.frames.len() {
                let img = rgba(&ase.visible(f), ase.width, ase.height);
                let at = IVec2::new(LEFT + (8 + f as i32) * TILE + 8, WALLS_Y);
                self.paint(self.l.wip, at, &img, &[], "walls2")?;
            }
            self.notes.push(format!(
                "walls2.aseprite has no PNG and nothing uses it; its {} frames are on the WIP layer",
                ase.frames.len()
            ));
        }
        Ok(())
    }

    /// One sheet: the PNG onto the art layer, hidden `.aseprite` layers onto the sketch layer.
    fn sheet(&mut self, name: &str, y: i32, frames: u32, sections: &[&str]) -> Result<()> {
        let png_path = self.root.join(format!("assets/images/{name}.png"));
        let img = png_io::decode(&std::fs::read(&png_path)?)
            .with_context(|| format!("decoding {}", png_path.display()))?
            .to_rgba();
        ensure!(
            (img.w, img.h) == (frames * TILE as u32, TILE as u32),
            "{name}.png is {}x{}, expected {frames} frames of {TILE}px",
            img.w,
            img.h
        );
        let at = IVec2::new(LEFT, y);
        self.paint(self.l.art, at, &img, sections, name)?;
        self.sheets.push((name.to_string(), at, img.clone()));

        let ase_path = self.root.join(format!("assets/images/{name}.aseprite"));
        if !ase_path.exists() {
            return Ok(());
        }
        let ase = aseprite::parse(&std::fs::read(&ase_path)?)
            .with_context(|| format!("reading {}", ase_path.display()))?;
        ensure!(ase.frames.len() == frames as usize, "{name}.aseprite has {} frames", ase.frames.len());
        let mut stale = Vec::new();
        for f in 0..ase.frames.len() {
            let from_ase = rgba(&ase.visible(f), ase.width, ase.height);
            let from_png = crop(&img, IRect::new(f as i32 * TILE, 0, TILE as u32, TILE as u32));
            if normalise(&from_ase) != normalise(&from_png) {
                stale.push(f);
            }
            for (l, layer) in ase.layers.iter().enumerate().filter(|(_, l)| !l.visible) {
                let pic = rgba(&ase.frames[f][l], ase.width, ase.height);
                let label = format!("{name} {}", layer.name);
                self.paint(self.l.sketch, at + IVec2::new(f as i32 * TILE, 0), &pic, sections, &label)?;
            }
        }
        if !stale.is_empty() {
            self.notes.push(format!(
                "{name}.png differs from {name}.aseprite in frames {stale:?}; the PNG (what the game draws) was used"
            ));
        }
        if !ase.tags.is_empty() {
            let tags: Vec<String> = ase.tags.iter().map(|(t, a, b)| format!("{t} {a}..={b}")).collect();
            self.notes.push(format!("{name}.aseprite tags (cross-check with sprite names): {}", tags.join(", ")));
        }
        for layer in ase.layers.iter().filter(|l| !l.visible) {
            self.notes.push(format!(
                "{name}.aseprite's hidden layer {:?} is on the Sketch layer (hidden)",
                layer.name
            ));
        }
        Ok(())
    }

    fn named(&self, section: &str, name: &str) -> u8 {
        self.p
            .palette
            .named(section, name)
            .unwrap_or_else(|| panic!("no {name:?} in {section:?}"))
    }

    /// Everything the game draws in code, as pixels.
    fn code_art(&mut self) -> Result<()> {
        // The net (`appearance.rs:151-199` `weave`): the player's own silhouette in mesh, strands
        // and holes on a one-pixel check that runs unbroken across the sheet.
        let strand = self.named("Net", "strand");
        let hole = self.named("Net", "hole");
        let player = IRect::new(LEFT, PLAYER_Y, 10 * TILE as u32, TILE as u32);
        for y in 0..player.h as i32 {
            for x in 0..player.w as i32 {
                let p = player.min() + IVec2::new(x, y);
                if self.grid(self.l.art).get(p) != 0 {
                    let v = if (x + y) % 2 == 0 { strand } else { hole };
                    self.grid(self.l.net).set(p, v);
                }
            }
        }

        // The bullet (`weapon.rs:479-480`): a 3x1 quad.
        let bullet = self.named("World", "bullet");
        self.grid(self.l.art).fill_rect(IRect::new(LEFT, BULLET_Y, 3, 1), bullet);

        // The blast (`weapon.rs:2558` `blast_radius`): a white disc, a frame per tick.
        let blast = self.named("World", "blast");
        let c = (BLAST as f32) / 2.0;
        for f in 0..EXPLOSION_TICKS {
            let r = blast_radius(f);
            let at = IVec2::new(BLAST_X + f as i32 * BLAST_STRIDE, FX_Y);
            for y in 0..BLAST as i32 {
                for x in 0..BLAST as i32 {
                    let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
                    // Strictly inside: a disc of radius 0 draws nothing, as the scaled mesh does.
                    if dx * dx + dy * dy < r * r {
                        self.grid(self.l.art).set(at + IVec2::new(x, y), blast);
                    }
                }
            }
        }

        // The flag pads (`props.rs:196`): the weapon mark under each team's colour, tinted the way
        // a sprite's colour tints it.
        self.p.palette.begin_section("Flag pads");
        let mark = strip(PROPS_Y, 0);
        for (team, shirt) in [("red", 12), ("blue", 4)] {
            let [r, g, b] = CLOTHING[shirt].1;
            let at = IVec2::new(LEFT + (if team == "red" { 5 } else { 6 }) * TILE, PROPS_Y);
            for y in 0..TILE {
                for x in 0..TILE {
                    let v = self.grid(self.l.art).get(mark.min() + IVec2::new(x, y));
                    if v == 0 {
                        continue;
                    }
                    let c = tint(self.p.palette.rgba(v), [r, g, b]);
                    let pal = &mut self.p.palette;
                    let t = match pal.find(c, Some("Flag pads")) {
                        Some(t) => t,
                        None => {
                            let n = pal.section("Flag pads").map_or(0, |s| s.len);
                            pal.push(format!("{team} {}", n / 2 + 1), c)?
                        }
                    };
                    self.grid(self.l.art).set(at + IVec2::new(x, y), t);
                }
            }
        }

        // The level editor's cursors (`editor.rs:1413-1562`), which it draws as flat quads.
        let hover = self.named("Editor", "hover");
        let selection = self.named("Editor", "selection");
        let problem = self.named("Editor", "problem");
        let art = self.l.art;
        let g = self.grid(art);
        g.fill_rect(strip(PROPS_Y, 8), hover);
        let sel = strip(PROPS_Y, 9);
        g.fill_rect(sel, selection);
        g.fill_rect(sel.grow(-1), 0);
        g.fill_rect(strip(PROPS_Y, 10), problem);

        // The sky and the flat fill are single colours; the level's backdrop shows the sky.
        Ok(())
    }

    fn add(&mut self, sprite: Sprite) {
        self.p.sprites.push(sprite);
    }

    fn sprites(&mut self) -> Result<()> {
        let art = vec![self.l.art];

        // The player: every pose by name, each with the pixel it holds a weapon by, plus the
        // cycles, the whole sheet by pose index, and the net over it.
        let pose = |i: usize| Frame::new(strip(PLAYER_Y, i as i32)).with_point("hand", hand_pixel(i));
        let poses: Vec<Frame> = (0..POSES.len()).map(pose).collect();
        self.add(tagged(
            Sprite::new("player/poses", art.clone(), poses),
            "frames",
            &POSES.join(","),
        ));
        for (i, name) in POSES.iter().enumerate() {
            if !matches!(*name, "run_1" | "run_2" | "crouch_walk") {
                self.add(Sprite::new(format!("player/{name}"), art.clone(), vec![pose(i)]));
            }
        }
        self.add(Sprite::new(
            "player/run",
            art.clone(),
            vec![pose(1).with_duration(RUN_FRAME_TIME), pose(2).with_duration(RUN_FRAME_TIME)],
        ));
        self.add(Sprite::new(
            "player/crouch_walk",
            art.clone(),
            vec![pose(8).with_duration(RUN_FRAME_TIME), pose(9).with_duration(RUN_FRAME_TIME)],
        ));
        self.add(Sprite::new(
            "player/net",
            vec![self.l.net],
            (0..POSES.len()).map(|i| Frame::new(strip(PLAYER_Y, i as i32))).collect(),
        ));

        // Weapons: seated on a hand by `trigger`, firing from `muzzle`, lying centred by `rest`.
        let weapon = |i: usize| {
            let (_, held, shoots, [dx, dy]) = WEAPONS[i];
            let mut f = Frame::new(strip(WEAPONS_Y, i as i32));
            if held {
                f = f.with_point("trigger", TRIGGER);
            }
            if shoots {
                f = f.with_point("muzzle", MUZZLE);
            }
            f.with_point("rest", IVec2::new(4 + dx, 4 - dy))
        };
        for (i, (name, ..)) in WEAPONS.iter().enumerate() {
            if matches!(*name, "shell_1" | "shell_2" | "rocket_1" | "rocket_2") {
                continue;
            }
            self.add(Sprite::new(format!("weapons/{name}"), art.clone(), vec![weapon(i)]));
        }
        let (ring, core) = (SHELL_RING_TICKS * TICK, SHELL_CORE_TICKS * TICK);
        self.add(Sprite::new(
            "weapons/shell",
            art.clone(),
            vec![weapon(3).with_duration(ring), weapon(4).with_duration(core)],
        ));
        self.add(Sprite::new(
            "weapons/rocket",
            art.clone(),
            vec![weapon(8).with_duration(ring), weapon(9).with_duration(core)],
        ));
        self.add(Sprite::new(
            "weapons/bullet",
            art.clone(),
            vec![Frame::new(IRect::new(LEFT, BULLET_Y, 3, 1))],
        ));

        // Props.
        let prop = |i: i32| Frame::new(strip(PROPS_Y, i));
        self.add(Sprite::new("props/weapon_mark", art.clone(), vec![prop(0)]));
        self.add(tagged(
            Sprite::new("props/jump_pad", art.clone(), vec![prop(1), prop(2)]),
            "frames",
            "rest,sprung",
        ));
        self.add(Sprite::new("props/spikes", art.clone(), vec![prop(3)]));
        self.add(Sprite::new("props/flag_pad_red", art.clone(), vec![prop(5)]));
        self.add(Sprite::new("props/flag_pad_blue", art.clone(), vec![prop(6)]));
        self.add(Sprite::new("editor/hover", art.clone(), vec![prop(8)]));
        self.add(Sprite::new("editor/selection", art.clone(), vec![prop(9)]));
        self.add(Sprite::new("editor/problem", art.clone(), vec![prop(10)]));

        // Emotes, in key order (`net.rs` `Emote`).
        for (i, name) in ["cry", "angry", "laugh", "cool"].iter().enumerate() {
            self.add(Sprite::new(
                format!("emotes/{name}"),
                art.clone(),
                vec![Frame::new(strip(EMOTES_Y, i as i32))],
            ));
        }

        // The wall sheet as `cladding.rs` addresses it: seven drawn frames and three transposes,
        // so an engine that can only flip gets every orientation.
        let mut walls: Vec<Frame> = (0..7).map(|i| Frame::new(strip(WALLS_Y, i))).collect();
        for i in [1, 2, 3] {
            walls.push(Frame::new(strip(WALLS_Y, i)).with_transform(D4::TRANSPOSE));
        }
        self.add(tagged(Sprite::new("walls/sheet", art.clone(), walls), "frames", &WALL_FRAMES.join(",")));

        // Platforms: ink only on the bottom five rows (`cladding.rs:111-118`).
        let ink = IRect::new(0, 3, TILE as u32, 5);
        self.add(tagged(
            Sprite::new(
                "platforms/sheet",
                art.clone(),
                (0..3).map(|i| Frame::new(strip(PLATFORMS_Y, i)).with_rect("ink", ink)).collect(),
            ),
            "frames",
            "cap_left,middle,cap_right",
        ));

        // The blast, one frame per tick, pivoting on its centre.
        let mut blast = Sprite::new(
            "fx/explosion",
            art,
            (0..EXPLOSION_TICKS)
                .map(|f| {
                    Frame::new(IRect::new(BLAST_X + f as i32 * BLAST_STRIDE, FX_Y, BLAST, BLAST))
                        .with_duration(TICK)
                })
                .collect(),
        );
        blast.pivot = Some(IVec2::splat(BLAST as i32 / 2));
        blast.tags.insert("radius".into(), "18 * sin(pi * tick / 8)".into());
        self.add(blast);

        // Recolour slots: the sheet's tones, and where a player picks replacements from.
        for (name, _) in PLAYER_KEYS {
            let options = if name == "skin" { "Skin tones" } else { "Clothing" };
            let index = self.named("Player keys", name);
            self.p.swaps.push(SwapSlot {
                name: name.into(),
                index,
                options: options.into(),
            });
        }
        Ok(())
    }

    fn terrains(&mut self) {
        let art = vec![self.l.art];
        self.p.tilesets.push(Tileset {
            name: "walls".into(),
            origin: IVec2::new(LEFT, WALLS_Y),
            tile_size: TILE as u32,
            columns: 7,
            count: 7,
            layers: art.clone(),
        });
        self.p.tilesets.push(Tileset {
            name: "platforms".into(),
            origin: IVec2::new(LEFT, PLATFORMS_Y),
            tile_size: TILE as u32,
            columns: 3,
            count: 3,
            layers: art,
        });
        self.p.terrains.push(Terrain {
            name: "walls".into(),
            tileset: "walls".into(),
            rules: presets::minimal_d4(MinimalD4 {
                corner: 0,
                edge: 1,
                strip_end: 2,
                strip: 3,
                interior: 4,
                inside: 5,
                double_inside: 6,
            }),
            fallback: Some(4),
        });
        self.p.terrains.push(Terrain {
            name: "platforms".into(),
            tileset: "platforms".into(),
            rules: presets::horizontal_run(0, 1, 2, 1),
            fallback: None,
        });
    }

    /// Paint the showcase and put the cast in it.
    fn level(&mut self) -> Result<()> {
        let walls = self.p.terrain_id("walls").unwrap();
        let girders = self.p.terrain_id("platforms").unwrap();
        let top = MOCK_Y / TILE;
        let width = LEVEL[0].len() as i32;
        let mut cells = Grid::new();
        for (y, row) in LEVEL.iter().enumerate() {
            ensure!(row.len() as i32 == width, "level row {y} is {} wide", row.len());
            for (x, c) in row.chars().enumerate() {
                let v = match c {
                    '#' => walls,
                    '=' => girders,
                    '.' => continue,
                    other => bail!("level cell {other:?}"),
                };
                cells.set(IVec2::new(x as i32, top + y as i32), v);
            }
        }
        if let LayerKind::Terrain(t) = &mut self.p.layer_mut(self.l.level).unwrap().kind {
            t.cells = cells;
        }
        let area = IRect::new(0, MOCK_Y, (width * TILE) as u32, (LEVEL.len() as i32 * TILE) as u32);
        let sky = self.named("World", "sky");
        let l = self.l.sky;
        self.grid(l).fill_rect(area, sky);

        let sheets = self.p.layer(self.l.art).unwrap().pixels().unwrap().bounds().unwrap();
        let sheets = sheets.union(&self.p.layer(self.l.wip).unwrap().pixels().unwrap().bounds().unwrap());
        self.p.bookmarks = vec![
            Bookmark { name: "Sheets".into(), rect: IRect::new(0, 0, sheets.max().x as u32 + 8, sheets.max().y as u32 + 8) },
            Bookmark { name: "Mockup".into(), rect: area },
        ];

        self.cast()
    }

    fn cast(&mut self) -> Result<()> {
        // Standing on the surface of row `row` of the level, in column `col`.
        let on = |col: i32, row: i32| IVec2::new(col * TILE, MOCK_Y + row * TILE - TILE);
        // Standing on a girder in row `row`: its ink starts three rows down.
        let on_girder = |col: i32, row: i32| IVec2::new(col * TILE, MOCK_Y + row * TILE + 3 - TILE);
        let at = |col: i32, row: i32| IVec2::new(col * TILE, MOCK_Y + row * TILE);

        let red = self.look(["near-black", "flag red", "near-black", "brown"], 0);
        let blue = self.look(["blond", "navy", "grey", "near-black"], 3);

        // Pads, marks, spikes and what lies on them.
        self.prop("props/flag_pad_red", 0, on(1, 13));
        self.prop("props/flag_pad_blue", 0, on(36, 11));
        self.prop("props/weapon_mark", 0, on(11, 13));
        self.resting("weapons/grenade_capped", on(11, 13));
        self.prop("props/weapon_mark", 0, on(28, 13));
        self.resting("weapons/rocket_launcher", on(28, 13));
        self.prop("props/weapon_mark", 0, on_girder(18, 11));
        self.resting("weapons/sword", on_girder(18, 11));
        self.prop("props/jump_pad", 0, on(13, 13));
        self.prop("props/jump_pad", 1, on(24, 13));
        for col in 15..=20 {
            self.prop("props/spikes", 0, on(col, 13));
        }

        // The cast.
        self.actor(IDLE, on(3, 13), false, red.clone(), Some("weapons/flag_red"), None);
        let shotgun = self.actor(
            RUN_1,
            on(9, 13),
            false,
            self.look(["blond", "green", "navy", "brown"], 2),
            Some("weapons/shotgun"),
            Some("emotes/angry"),
        );
        if let Some(muzzle) = shotgun {
            for k in 0..3 {
                self.actor_sprite("weapons/bullet", 0, muzzle + IVec2::new(4 + k * 9, 0), D4::IDENTITY);
            }
        }
        let rocket = self.actor(
            JUMP_ARMED,
            at(12, 9),
            false,
            self.look(["rust", "purple", "near-black", "grey"], 4),
            Some("weapons/rocket_launcher"),
            None,
        );
        if let Some(muzzle) = rocket {
            self.actor_sprite("weapons/rocket", 0, muzzle + IVec2::new(30, -4), D4::IDENTITY);
        }
        self.actor_sprite(
            "fx/explosion",
            4,
            IVec2::new(15 * TILE + 4, MOCK_Y + 5 * TILE + 4) - IVec2::splat(BLAST as i32 / 2),
            D4::IDENTITY,
        );
        self.actor(
            WALL_SLIDE_ARMED,
            at(30, 11),
            true,
            self.look(["near-black", "cyan", "grey", "near-black"], 6),
            Some("weapons/handgun"),
            None,
        );
        let launcher = self.actor(
            CROUCH,
            on(34, 13),
            true,
            self.look(["brown", "yellow", "navy", "brown"], 1),
            Some("weapons/grenade_launcher"),
            None,
        );
        if let Some(muzzle) = launcher {
            self.actor_sprite("weapons/shell", 0, muzzle + IVec2::new(-20, -14), D4::IDENTITY);
        }
        let netter = self.actor(
            SLIDE,
            on(21, 13),
            false,
            self.look(["pink", "white", "purple", "near-black"], 5),
            Some("weapons/net_gun"),
            None,
        );
        if let Some(muzzle) = netter {
            self.actor_sprite("weapons/net", 0, muzzle + IVec2::new(10, -4), D4::IDENTITY);
        }
        // Caught: the body in its colours, the net over it from its own layer.
        let caught = on(26, 13);
        self.actor(IDLE, caught, true, self.look(["near-black", "grey", "navy", "brown"], 7), None, Some("emotes/cry"));
        self.actor_sprite("player/net", IDLE, caught, D4::FLIP_X);
        self.actor(IDLE, on_girder(5, 2), false, self.look(["blond", "rust", "navy", "brown"], 0), Some("weapons/sword"), Some("emotes/laugh"));
        self.actor(IDLE, on(37, 11), true, blue, Some("weapons/flag_blue"), Some("emotes/cool"));
        self.actor(JUMP, at(20, 4), true, self.look(["grey", "near-black", "green", "brown"], 2), None, None);
        Ok(())
    }

    fn look(&self, clothes: [&str; 4], skin: usize) -> Vec<(String, u8)> {
        let mut out: Vec<(String, u8)> = ["hair", "shirt", "pants", "shoes"]
            .iter()
            .zip(clothes)
            .map(|(slot, name)| (slot.to_string(), self.named("Clothing", name)))
            .collect();
        let skin = self.p.palette.section("Skin tones").unwrap().start as u8 + skin as u8;
        out.push(("skin".into(), skin));
        out
    }

    fn instances(&mut self, layer: LayerId) -> &mut Vec<Instance> {
        match &mut self.p.layer_mut(layer).unwrap().kind {
            LayerKind::Instances(v) => v,
            _ => unreachable!("an instance layer"),
        }
    }

    fn prop(&mut self, sprite: &str, frame: u32, pos: IVec2) {
        let l = self.l.props;
        self.instances(l).push(Instance {
            sprite: sprite.into(),
            frame,
            pos,
            transform: D4::IDENTITY,
            swaps: Vec::new(),
        });
    }

    /// A weapon lying loose in a cell, pulled back to its middle by its `rest` point.
    fn resting(&mut self, sprite: &str, cell: IVec2) {
        let rest = self.point(sprite, 0, "rest").unwrap_or(IVec2::splat(4));
        self.prop(sprite, 0, cell - (rest - IVec2::splat(4)));
    }

    fn actor_sprite(&mut self, sprite: &str, frame: u32, pos: IVec2, transform: D4) {
        let l = self.l.actors;
        self.instances(l).push(Instance {
            sprite: sprite.into(),
            frame,
            pos,
            transform,
            swaps: Vec::new(),
        });
    }

    fn point(&self, sprite: &str, frame: usize, name: &str) -> Option<IVec2> {
        self.p.sprite(sprite)?.frames.get(frame)?.points.get(name).copied()
    }

    /// A player in a pose, holding a weapon by the file's own anchors: the pose's `hand` on the
    /// weapon's `trigger`, both mirrored for a player facing left. Returns the muzzle, if any, in
    /// canvas pixels.
    fn actor(
        &mut self,
        pose: u32,
        pos: IVec2,
        left: bool,
        look: Vec<(String, u8)>,
        weapon: Option<&str>,
        emote: Option<&str>,
    ) -> Option<IVec2> {
        let flip = if left { D4::FLIP_X } else { D4::IDENTITY };
        let mirror = |p: IVec2| if left { IVec2::new(TILE - 1 - p.x, p.y) } else { p };
        let l = self.l.actors;
        self.instances(l).push(Instance {
            sprite: "player/poses".into(),
            frame: pose,
            pos,
            transform: flip,
            swaps: look,
        });
        if let Some(emote) = emote {
            self.actor_sprite(emote, 0, pos - IVec2::new(0, TILE + 1), D4::IDENTITY);
        }
        let weapon = weapon?;
        let hand = mirror(self.point("player/poses", pose as usize, "hand")?);
        let trigger = mirror(self.point(weapon, 0, "trigger")?);
        let at = pos + hand - trigger;
        self.actor_sprite(weapon, 0, at, flip);
        self.point(weapon, 0, "muzzle").map(|m| at + mirror(m))
    }

    /// Every sheet comes back out of the file exactly, and the wall terrain draws what
    /// `cladding.rs` would.
    fn verify(&self) -> Result<()> {
        let mut r = Renderer::new(&self.p);
        for (name, at, png) in &self.sheets {
            let rect = IRect::new(at.x, at.y, png.w, png.h);
            let back = r.indexed(rect, &[self.l.art]).to_rgba(&self.p.palette);
            ensure!(normalise(&back) == normalise(png), "{name}.png does not survive the round trip");
        }

        // `derive_wall_sheet`: the drawn frames, then TOP, COLUMN_HEAD and COLUMN_BODY transposed.
        let (_, _, walls) = self.sheets.iter().find(|(n, ..)| n == "walls").context("walls")?;
        for f in 0..10 {
            let got = r.frame("walls/sheet", f).unwrap().to_rgba(&self.p.palette);
            let src = if f < 7 { f } else { [1, 2, 3][f - 7] };
            let mut want = crop(walls, IRect::new(src as i32 * TILE, 0, TILE as u32, TILE as u32));
            if f >= 7 {
                want = transpose(&want);
            }
            ensure!(normalise(&got) == normalise(&want), "walls/sheet frame {f} ({})", WALL_FRAMES[f]);
        }

        // Every frame of every sheet is reachable by some sprite name.
        let mut covered: BTreeMap<(i32, i32), ()> = BTreeMap::new();
        for s in &self.p.sprites {
            for f in &s.frames {
                covered.insert((f.rect.x, f.rect.y), ());
            }
        }
        for (name, at, png) in &self.sheets {
            for f in 0..(png.w as i32 / TILE) {
                ensure!(
                    covered.contains_key(&(at.x + f * TILE, at.y)),
                    "{name}.png frame {f} has no sprite"
                );
            }
        }

        // Explosion frames are exactly as wide as the disc the game scales.
        for f in 0..EXPLOSION_TICKS as usize {
            let pic = r.frame("fx/explosion", f).unwrap();
            let across = (0..BLAST).filter(|&x| pic.get(x, BLAST / 2) != 0).count() as f32;
            let r = blast_radius(f as u32);
            ensure!((across - 2.0 * r).abs() <= 1.5, "blast frame {f}: {across}px across for radius {r}");
        }
        Ok(())
    }
}

/// `weapon.rs:2558`.
fn blast_radius(age: u32) -> f32 {
    if age >= EXPLOSION_TICKS {
        return 0.0;
    }
    EXPLOSION_RADIUS * (age as f32 / EXPLOSION_TICKS as f32 * std::f32::consts::PI).sin()
}

fn tagged(mut s: Sprite, key: &str, value: &str) -> Sprite {
    s.tags.insert(key.into(), value.into());
    s
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
