# Pixelle

An indexed pixel-art editor where a project is a folder. A Bevy game reads that folder directly, with no export step.

Each sprite and each terrain is a text file of its own, so a change to one sprite touches one file and a few changed pixels are a few changed lines in git. All files share the project's palette. The folder opens as one project:

```text
art/
  project.pxl        the palette (labelled sections, up to 256 colours), recolour slots, the list of files
  player.pxl         a sprite: layers, frames, timings, anchor points, pixels as rows of palette indices
  walls.pxl          a terrain: a preset and the tiles it needs
  wip/walls2.pxl
```

- **Sprites:** a still, an animation or a sheet. Frames can be named for code, have their own size and timing, and carry named points (`muzzle`, `hand`) and rects. Layers can be left out of the export, for sketches.
- **Animations:** named sequences of a sheet's frames, each with its own duration, e.g. `"run": [(1, 0.11), (2, 0.11)]` in the sprite's file. The game plays one as a sprite of its own, `art::player::anims::RUN`. Its frames share the sheet's place in the atlas, so an animation costs no extra pixels.
- **Terrains:** "2D grid, two surfaces", ground against air. A cell shows the tile that matches the air around it, or four quarters cut from the tiles' own corners. With the mirror off, which is the only option for now, every way a tile faces is drawn on its own (21 tiles), so tops can be lit and bottoms shaded.
- **Recolour slots:** palette indices the game swaps at runtime.

Every file uses the `.pxl` extension; the first word inside says what it is (`Project(`, `Sprite(`, `Terrain(`). A pixel is two characters, its palette index in hex, with `..` for transparent:

```text
"edge top": [
  "0b0b0b0b0b0b0b0b",
  "0404040404040404",
  "0404040404040405",
  ...
```

## Crates

| Path | What |
|---|---|
| `crates/pixelle-core` | The folder format (`folder`), compositing, terrain rules and presets (`two_surface`, `minimal_d4`, `edge16`, `blob47`), atlas packing, the baked `.pxlb` format, PNG/Aseprite/palette import. No UI or engine dependencies. |
| `crates/pixelle-editor` | The editor (eframe/egui): a file tree, a tab per open file, and the Sandbox. The canvas renders through a swappable `CanvasBackend`; today that is CPU tiles. |
| `crates/pixelle-cli` | `pixelle`: `new`, `info`, `bake`, `atlas`, `sheet`, `codegen`, `import-png`, `import-aseprite`, `import-palette`. |
| `crates/pixelle-build` | For build.rs: generates typed constants (`art::PLAYER`, `art::player::CROUCH`, `art::player::anims::RUN`, `art::points::MUZZLE`, `art::palette::clothing::RUST`) and bakes the web file. |
| `crates/bevy_pixelle` | Bevy 0.19 asset loader for a project folder (`project.pxl`, hot-reloading any of its files) and for `.pxlb`. Provides `PixelleSprite` (frames, recolouring, hot reload), `PixelleAnimation`, anchor offsets, runtime terrain autotiling (`art.terrain(..).cell(..)`), and each sprite as a sheet image (`project.pxl#sheet/player`). |
| `examples/showcase` | run-2d's art in Bevy, driven entirely by the generated names. |
| `tools/run2d-import` | Turns run-2d's sheets into a project folder, with pixel-exact round-trip checks. |

## Try it

```sh
cargo run -p pixelle-editor -- examples/showcase/assets/run2d     # edit a project
cargo run -p showcase                                            # see it in Bevy; saving in the editor hot-reloads
cargo run -p pixelle-cli -- info examples/showcase/assets/run2d
```

### Editor

- **Files:** the folder's files are in a tree on the left. ⌘N makes a sprite or a terrain; right-click a file to rename, duplicate, delete or reveal it. Each open file is a tab.
- **Saving:** ⌘S writes every file that changed and nothing else. Renames and deletes reach the disk at the same time.
- **The Sandbox:** the first tab, always open. It's an infinite canvas for trying the project's terrains with the Terrain tool and for doodling. It is never saved.
- **Sprite tabs:** frames sit in a strip, and painting stays inside them. Frames are added, duplicated, reordered, resized and named in the inspector; `[` and `]` step through them. The sheet's animations play in a column to its left. The inspector's Animations section makes them from the selected frame, then edits their steps (frame and duration), order and names, with a preview of each.
- **Terrain tabs:** the tiles are labelled by what they are ("corner top-left", "inside bottom-right"). Below them is a patch of ground drawn with them, which you paint with `T`.
- **The palette** belongs to the project. Inserting or deleting a colour remaps every file, and is its own undo step.

**Keys**

- **Tools:** `B` pencil, `E` eraser, `L` line, `R` rectangle (Shift: filled), `G` fill (Shift: dither), `I` picker, `M` select, `T` terrain, `S` frames (select, resize from the corner, place points), `H` hand.
- **Colours:** `X` swaps primary and secondary. Alt-click picks.
- **Moving around:** Space or the middle button pans. ⌘-scroll or pinch zooms; `+`/`-` step the zoom; ⌘0 fits.
- **Undo:** ⌘Z / ⇧⌘Z.
- **Selections:** ⌘C / ⌘X / ⌘V; arrows nudge; Enter puts pixels down; Esc deselects.
- **Tabs:** ⌘W closes the tab in front.

Unsaved work is autosaved to `.pixelle/autosave/` in the project every 30 s and offered back on reopen; `.pixelle/` carries its own `.gitignore`. A file changed by something else is reloaded, or you're asked first if you have unsaved changes to it.

`pixelle-editor <folder> --open <file> --screenshot out.png` (plus `--zoom`, `--tool`, `--layer`) renders the editor once and quits, for scripted checks.

## In a game

```rust
// build.rs
pixelle_build::Art::new("assets/art").bake_to("assets/art.pxlb").run().unwrap();

// main.rs
include!(concat!(env!("OUT_DIR"), "/art.rs"));
app.add_plugins(bevy_pixelle::PixellePlugin);
let art = assets.load(if cfg!(target_arch = "wasm32") { "art.pxlb" } else { "art/project.pxl" });
commands.spawn((
    PixelleSprite::new(art.clone(), art::PLAYER)
        .frame(art::player::IDLE)
        .swap(art::swaps::SHIRT, art::palette::clothing::RUST),
    Transform::default(),
));
```

Code that cuts its own rectangles out of sheet images can load a sprite as one: `assets.load("art/project.pxl#sheet/player")` is its frames side by side, as a hand-exported PNG sheet would be.

Grid maps made at run time are clad by the project's own terrain:

```rust
let walls = art.terrain(art::terrains::WALLS).unwrap();
for piece in walls.cell(|d| is_ground(cell + d)) {          // d in cells, y up
    commands.spawn((walls.sprite(&piece), Transform::from_translation((centre + piece.offset).extend(z))));
}
```

The baked `.pxlb` is only for the web, where a folder of text files would mean many larger downloads. build.rs writes it; don't commit it.

## The baked file

**`.pxlb`** starts with `b"PXLB"`, then a version and flags. After that come the palette, the recolour slots, a deflated indexed atlas, each sprite's frames, timings, anchors and tags, and each terrain's rules and tiles. run-2d's art bakes to a few KB.
