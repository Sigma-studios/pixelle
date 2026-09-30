# Pixelle

An indexed pixel-art editor built on one infinite canvas. A Bevy game loads the project file directly.

You draw every piece of a game's art side by side: sheets, tiles, and a mockup level made from them.

- **Sprites:** named rectangles cut from the canvas. They can be limited to particular layers, and hold animation timings and named anchor points (`muzzle`, `hand`, …).
- **Tilesets:** regions of the canvas. **Terrains** autotile a painted grid from them by rules, so editing a tile updates every level that uses it.
- **Instances:** live copies of sprites for mockups.
- **Palette:** one palette of up to 256 colours in labelled sections.
- **Recolour slots:** palette indices the game swaps at runtime.

Everything lives in one `.pxl` file.

## Crates

| Path | What |
|---|---|
| `crates/pixelle-core` | Data model, compositing, terrain rules (whole tiles in 8 orientations, with a quarter-tile fallback; `minimal_d4`, `edge16`, `blob47` presets), atlas packing, `.pxl` and baked `.pxlb` formats, PNG/Aseprite/palette import. No UI or engine dependencies. |
| `crates/pixelle-editor` | The editor (eframe/egui). The canvas renders through a swappable `CanvasBackend`; today that is CPU tiles. |
| `crates/pixelle-cli` | `pixelle`: `new`, `info`, `render`, `pack`, `bake`, `codegen`, `import-png`, `import-aseprite`, `import-palette`. |
| `crates/pixelle-build` | For build.rs: bakes the runtime file and generates typed constants (`art::weapons::SHOTGUN`, `art::player::poses::CROUCH`, `art::points::MUZZLE`, `art::palette::clothing::RUST`). |
| `crates/bevy_pixelle` | Bevy 0.19 asset loader for `.pxl` and `.pxlb`. Provides `PixelleSprite` (frames, recolouring, hot reload), `PixelleAnimation`, anchor offsets, and runtime terrain autotiling (`art.terrain(..).cell(..)`). |
| `examples/showcase` | run-2d's art in Bevy, driven entirely by the generated names. |
| `tools/run2d-import` | Generates `run2d.pxl` from the run-2d game, with pixel-exact round-trip checks. |

## Try it

```sh
cargo run -p run2d-import -- ../../bevy/run-2d -o out/run2d.pxl   # regenerate the file
cargo run -p pixelle-editor -- out/run2d.pxl                      # edit it
cargo run -p showcase                                             # see it in Bevy (hot-reloads the .pxl)
cargo run -p pixelle-cli -- info out/run2d.pxl
```

### Editor keys

- **Tools:** `B` pencil, `E` eraser, `L` line, `R` rectangle (Shift: filled), `G` fill (Shift: dither), `I` picker, `M` select, `T` terrain, `S` sprite, `N` instance, `H` hand.
- **Colours:** `X` swaps primary and secondary. Alt-click picks a colour with any paint tool.
- **Moving around:** Space or the middle button pans. ⌘-scroll or pinch zooms; `+`/`-` step the zoom; ⌘0 fits everything.
- **Undo:** ⌘Z / ⇧⌘Z.
- **Selections:** ⌘C / ⌘X / ⌘V; arrows nudge; Enter puts pixels down; Esc deselects.
- **Instances:** `F` flips; `[` and `]` step through frames.

Fill floods the whole connected region; transparent space fills only when enclosed or inside a selection, since the canvas is infinite. Animations play beside their strips on the canvas. Undo lasts for the session. Unsaved work is autosaved beside the file every 30 s and offered back on reopen. A file changed by something else is reloaded, or you're asked first if you have unsaved changes.

`--screenshot out.png` (plus `--view`, `--zoom`, `--select`, `--tool`, `--layer`) renders the editor once and quits, for scripted checks.

## In a game

```rust
// build.rs
pixelle_build::Art::new("assets/art.pxl").bake_to("assets/art.pxlb").run().unwrap();

// main.rs
include!(concat!(env!("OUT_DIR"), "/art.rs"));
app.add_plugins(bevy_pixelle::PixellePlugin);
let art = assets.load(if cfg!(debug_assertions) { "art.pxl" } else { "art.pxlb" });
commands.spawn((
    PixelleSprite::new(art.clone(), art::player::POSES)
        .frame(art::player::poses::IDLE)
        .swap(art::swaps::SHIRT, art::palette::clothing::RUST),
    Transform::default(),
));
```

Grid maps made at run time (a level editor in the game) are clad by the file's own terrain rules:

```rust
let walls = art.terrain(art::terrains::WALLS).unwrap();
for piece in walls.cell(|d| is_ground(cell + d)) {          // d in cells, y up
    commands.spawn((walls.sprite(&piece), Transform::from_translation((centre + piece.offset).extend(z))));
}
```

Only the project is tracked. The baked file is derived at build time, so don't commit it.

## File formats

- **`.pxl`** (the project):
  - `b"PXLL"` magic, then a u16 version (2; version 1 files still load) and u32 flags (both little-endian), then the project, postcard-encoded.
  - Pixels are stored as sparse 64×64 chunks, each deflate-compressed.
  - `pixelle info --ron` dumps everything except the pixels as readable text.
- **`.pxlb`** (the baked runtime file):
  - `b"PXLB"`, version and flags, then the palette, recolour slots, a deflated indexed atlas, and each sprite's frames, timings, anchors and tags, and each terrain's rules and tiles.
  - run-2d's art bakes to about 6 KB.
