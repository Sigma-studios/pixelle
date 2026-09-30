# Pixelle

An indexed pixel-art editor built on one infinite canvas. A game loads the project file directly.

You draw every piece of a game's art side by side: sheets, tiles, and a mockup level made from them. Named **sprites** are cut from the canvas, and each one can be limited to particular layers. **Tilesets** are regions of the canvas, and **terrains** autotile a painted grid from them, so editing a tile updates every level that uses it. **Swap slots** mark palette indices the game recolours at runtime. Everything lives in one `.pxl` file.

## Crates

| Path | What |
|---|---|
| `crates/pixelle-core` | Data model, compositing, terrain rules (D4 autotiling), atlas packing, `.pxl` I/O. No UI, no engine dependencies. |
| `crates/pixelle-cli` | The `pixelle` command: `info`, `render`, `pack`. |
| `tools/run2d-import` | Generates `run2d.pxl` from the run-2d game: its sheets, the art its code draws, and a showcase level. |

Planned next: `pixelle-editor` (eframe), then `bevy_pixelle` plus `pixelle-build` (typed consts via build.rs).

## Try it

```sh
cargo run -p run2d-import -- ../../bevy/run-2d -o out/run2d.pxl
cargo run -p pixelle-cli -- info   out/run2d.pxl
cargo run -p pixelle-cli -- render out/run2d.pxl -o out/mockup.png --scale 4 --rect Mockup
cargo run -p pixelle-cli -- pack   out/run2d.pxl -o out/atlas.png
```

Before it writes anything, the importer checks two things:
- every sheet comes back out of the file pixel for pixel;
- the wall terrain reproduces the sheet that `cladding.rs` derives at runtime.

## File format

- `b"PXLL"` magic, then a u16 version and u32 flags (both little-endian).
- Then the project, postcard-encoded.
- Pixel data is stored as sparse 64×64 chunks, each deflate-compressed.
- `pixelle info --ron` dumps everything except the pixels as readable text.
