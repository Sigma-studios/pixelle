//! Compositing layers into pictures.
//!
//! Two kinds of output. An [`Indexed`] picture is what gets exported: palette indices, the topmost
//! non-transparent layer winning each pixel. An [`RgbaImage`] is what gets looked at: every visible
//! layer blended over the one below in its palette colours, translucent entries included.

use std::collections::HashMap;

use crate::d4::D4;
use crate::geom::{IRect, IVec2};
use crate::layer::{Instance, Layer, LayerId, LayerKind};
use crate::palette::Palette;
use crate::project::Project;
use crate::tiles::Choice;

/// How deep instances of sprites whose layers hold instances may nest before drawing stops.
const MAX_DEPTH: u32 = 4;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Indexed {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

impl Indexed {
    pub fn new(w: u32, h: u32) -> Self {
        Self {
            w,
            h,
            px: vec![0; (w * h) as usize],
        }
    }

    pub fn get(&self, x: u32, y: u32) -> u8 {
        self.px[(y * self.w + x) as usize]
    }

    pub fn set(&mut self, x: u32, y: u32, v: u8) {
        self.px[(y * self.w + x) as usize] = v;
    }

    /// This picture in another orientation.
    pub fn transformed(&self, t: D4) -> Indexed {
        if t.is_identity() {
            return self.clone();
        }
        let (w, h) = t.size(self.w, self.h);
        let mut out = Indexed::new(w, h);
        for y in 0..self.h {
            for x in 0..self.w {
                let to = t.apply(IVec2::new(x as i32, y as i32), self.w, self.h);
                out.set(to.x as u32, to.y as u32, self.get(x, y));
            }
        }
        out
    }

    /// The `w`×`h` picture at `(x, y)` of this one.
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Indexed {
        let mut out = Indexed::new(w, h);
        for dy in 0..h.min(self.h.saturating_sub(y)) {
            for dx in 0..w.min(self.w.saturating_sub(x)) {
                out.set(dx, dy, self.get(x + dx, y + dy));
            }
        }
        out
    }

    /// Replace indices through a lookup table.
    pub fn remap(&mut self, table: &[u8; 256]) {
        for v in &mut self.px {
            *v = table[*v as usize];
        }
    }

    /// Draw `src` with its top-left at `at`, skipping transparent pixels and anything off the edge.
    pub fn blit(&mut self, src: &Indexed, at: IVec2) {
        for y in 0..src.h as i32 {
            let ty = at.y + y;
            if ty < 0 || ty >= self.h as i32 {
                continue;
            }
            for x in 0..src.w as i32 {
                let tx = at.x + x;
                if tx < 0 || tx >= self.w as i32 {
                    continue;
                }
                let v = src.get(x as u32, y as u32);
                if v != 0 {
                    self.set(tx as u32, ty as u32, v);
                }
            }
        }
    }

    pub fn is_blank(&self) -> bool {
        self.px.iter().all(|&v| v == 0)
    }

    pub fn to_rgba(&self, palette: &Palette) -> RgbaImage {
        let mut out = RgbaImage::new(self.w, self.h);
        for (i, &v) in self.px.iter().enumerate() {
            out.px[i * 4..i * 4 + 4].copy_from_slice(&palette.rgba(v));
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

impl RgbaImage {
    pub fn new(w: u32, h: u32) -> Self {
        Self {
            w,
            h,
            px: vec![0; (w * h * 4) as usize],
        }
    }

    pub fn filled(w: u32, h: u32, rgba: [u8; 4]) -> Self {
        Self {
            w,
            h,
            px: rgba.repeat((w * h) as usize),
        }
    }

    /// Source-over blend of one straight-alpha colour onto pixel `i`.
    fn blend(&mut self, i: usize, src: [u8; 4]) {
        let sa = src[3] as f32 / 255.0;
        if sa <= 0.0 {
            return;
        }
        let dst = &mut self.px[i * 4..i * 4 + 4];
        if sa >= 1.0 {
            dst.copy_from_slice(&src);
            return;
        }
        let da = dst[3] as f32 / 255.0;
        let oa = sa + da * (1.0 - sa);
        for c in 0..3 {
            let v = (src[c] as f32 * sa + dst[c] as f32 * da * (1.0 - sa)) / oa;
            dst[c] = v.round() as u8;
        }
        dst[3] = (oa * 255.0).round() as u8;
    }

    /// Scale up by a whole number, nearest neighbour.
    pub fn scaled(&self, k: u32) -> RgbaImage {
        if k <= 1 {
            return self.clone();
        }
        let mut out = RgbaImage::new(self.w * k, self.h * k);
        for y in 0..out.h {
            for x in 0..out.w {
                let s = ((y / k) * self.w + x / k) as usize * 4;
                let d = (y * out.w + x) as usize * 4;
                out.px[d..d + 4].copy_from_slice(&self.px[s..s + 4]);
            }
        }
        out
    }
}

/// Renders parts of a project, remembering tiles it has already drawn.
pub struct Renderer<'a> {
    project: &'a Project,
    tiles: HashMap<(usize, u32), Indexed>,
}

impl<'a> Renderer<'a> {
    pub fn new(project: &'a Project) -> Self {
        Self {
            project,
            tiles: HashMap::new(),
        }
    }

    /// The listed layers over `rect`, as indices. Layers draw in project order whatever order they
    /// are listed in.
    pub fn indexed(&mut self, rect: IRect, layers: &[LayerId]) -> Indexed {
        self.indexed_at_depth(rect, layers, 0)
    }

    fn indexed_at_depth(&mut self, rect: IRect, layers: &[LayerId], depth: u32) -> Indexed {
        let mut out = Indexed::new(rect.w, rect.h);
        if depth > MAX_DEPTH {
            return out;
        }
        let project = self.project;
        for layer in project.layers.iter().filter(|l| layers.contains(&l.id)) {
            self.draw_layer(layer, rect, depth, &mut out);
        }
        out
    }

    /// One frame of a sprite, as exported: its layers composited, then oriented.
    pub fn frame(&mut self, sprite: &str, frame: usize) -> Option<Indexed> {
        self.frame_at_depth(sprite, frame, 0)
    }

    fn frame_at_depth(&mut self, sprite: &str, frame: usize, depth: u32) -> Option<Indexed> {
        let sprite = self.project.sprite(sprite)?;
        let f = sprite.frames.get(frame)?;
        Some(
            self.indexed_at_depth(f.rect, &sprite.layers, depth)
                .transformed(f.transform),
        )
    }

    /// Every visible layer over `rect`, blended in palette colour over `background`.
    pub fn rgba(&mut self, rect: IRect, background: [u8; 4]) -> RgbaImage {
        let mut out = RgbaImage::filled(rect.w, rect.h, background);
        let project = self.project;
        for layer in project.layers.iter().filter(|l| l.visible) {
            // Instances blend one at a time, so a translucent one over another shows it through.
            let pics: Vec<Indexed> = match &layer.kind {
                LayerKind::Instances(list) => list
                    .iter()
                    .map(|inst| {
                        let mut pic = Indexed::new(rect.w, rect.h);
                        self.draw_instance(inst, rect, 0, &mut pic);
                        pic
                    })
                    .collect(),
                _ => {
                    let mut pic = Indexed::new(rect.w, rect.h);
                    self.draw_layer(layer, rect, 0, &mut pic);
                    vec![pic]
                }
            };
            for pic in pics {
                for (i, &v) in pic.px.iter().enumerate() {
                    if v != 0 {
                        out.blend(i, project.palette.rgba(v));
                    }
                }
            }
        }
        out
    }

    fn tile(&mut self, tileset: usize, index: u32, depth: u32) -> Indexed {
        if let Some(t) = self.tiles.get(&(tileset, index)) {
            return t.clone();
        }
        let ts = &self.project.tilesets[tileset];
        let pic = self.indexed_at_depth(ts.tile_rect(index), &ts.layers.clone(), depth + 1);
        self.tiles.insert((tileset, index), pic.clone());
        pic
    }

    fn draw_layer(&mut self, layer: &Layer, rect: IRect, depth: u32, out: &mut Indexed) {
        let project = self.project;
        match &layer.kind {
            LayerKind::Pixels(grid) => {
                for y in 0..rect.h {
                    for x in 0..rect.w {
                        let v = grid.get(IVec2::new(rect.x + x as i32, rect.y + y as i32));
                        if v != 0 {
                            out.set(x, y, v);
                        }
                    }
                }
            }
            LayerKind::Terrain(t) => {
                let s = t.cell_size as i32;
                let min = IVec2::new(rect.x.div_euclid(s), rect.y.div_euclid(s));
                let max = IVec2::new(
                    (rect.max().x + s - 1).div_euclid(s),
                    (rect.max().y + s - 1).div_euclid(s),
                );
                for cy in min.y..max.y {
                    for cx in min.x..max.x {
                        let cell = IVec2::new(cx, cy);
                        let v = t.cells.get(cell);
                        if v == 0 {
                            continue;
                        }
                        let Some(terrain) = project.terrains.get(v as usize - 1) else {
                            continue;
                        };
                        let Some(ts) = project.tilesets.iter().position(|ts| ts.name == terrain.tileset)
                        else {
                            continue;
                        };
                        let Some(choice) = terrain.choose(|d| t.cells.get(cell + d) == v) else {
                            continue;
                        };
                        match choice {
                            Choice::Whole { tile, transform } => {
                                let pic = self.tile(ts, tile, depth).transformed(transform);
                                out.blit(&pic, cell * s - rect.min());
                            }
                            Choice::Quarters(quarters) => {
                                let h = s / 2;
                                for (q, corner) in quarters.iter().zip([IVec2::ZERO, IVec2::new(h, 0), IVec2::new(0, h), IVec2::new(h, h)]) {
                                    let whole = self.tile(ts, q.tile, depth).transformed(q.transform);
                                    let piece = whole.crop(0, 0, h as u32, h as u32).transformed(q.place);
                                    out.blit(&piece, cell * s + corner - rect.min());
                                }
                            }
                        }
                    }
                }
            }
            LayerKind::Instances(list) => {
                for inst in list {
                    self.draw_instance(inst, rect, depth, out);
                }
            }
        }
    }

    fn draw_instance(&mut self, inst: &Instance, rect: IRect, depth: u32, out: &mut Indexed) {
        let Some(mut pic) = self.frame_at_depth(&inst.sprite, inst.frame as usize, depth + 1) else {
            return;
        };
        if !inst.swaps.is_empty() {
            pic.remap(&swap_table(self.project, &inst.swaps));
        }
        out.blit(&pic.transformed(inst.transform), inst.pos - rect.min());
    }
}

/// The lookup table that recolours a picture through swap slots, `(slot name, new index)`.
/// Unknown slots are ignored.
pub fn swap_table(project: &Project, swaps: &[(String, u8)]) -> [u8; 256] {
    let mut table = [0u8; 256];
    for (i, v) in table.iter_mut().enumerate() {
        *v = i as u8;
    }
    for (name, to) in swaps {
        if let Some(slot) = project.swap(name) {
            table[slot.index as usize] = *to;
        }
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Grid;
    use crate::layer::{Instance, TerrainLayer};
    use crate::sprite::{Frame, Sprite};
    use crate::tiles::{Terrain, Tileset, presets};

    fn project() -> (Project, LayerId) {
        let mut p = Project::new("test");
        p.palette.begin_section("S");
        p.palette.push("a", [255, 0, 0, 255]).unwrap();
        p.palette.push("b", [0, 0, 255, 128]).unwrap();
        let mut g = Grid::new();
        // A 2x2 tile: index 1 at the top-left, 2 elsewhere.
        g.fill_rect(IRect::new(0, 0, 2, 2), 2);
        g.set(IVec2::new(0, 0), 1);
        let art = p.add_layer("art", LayerKind::Pixels(g));
        (p, art)
    }

    #[test]
    fn frames_export_in_their_orientation() {
        let (mut p, art) = project();
        p.sprites.push(Sprite::new(
            "t",
            vec![art],
            vec![Frame::new(IRect::new(0, 0, 2, 2)).with_transform(D4::FLIP_X)],
        ));
        let f = Renderer::new(&p).frame("t", 0).unwrap();
        assert_eq!(f.px, vec![2, 1, 2, 2]);
    }

    #[test]
    fn terrain_draws_live_tiles() {
        let (mut p, art) = project();
        p.tilesets.push(Tileset {
            name: "ts".into(),
            origin: IVec2::ZERO,
            tile_size: 2,
            columns: 1,
            count: 1,
            layers: vec![art],
            gap: 0,
        });
        p.terrains.push(Terrain {
            name: "t".into(),
            tileset: "ts".into(),
            rules: presets::horizontal_run(0, 0, 0, 0),
            fallback: None,
            quarters: Vec::new(),
        });
        let mut cells = Grid::new();
        cells.set(IVec2::new(5, 0), 1);
        p.add_layer("level", LayerKind::Terrain(TerrainLayer { cell_size: 2, cells }));
        let level = p.layers[1].id;
        let pic = Renderer::new(&p).indexed(IRect::new(10, 0, 2, 2), &[level]);
        assert_eq!(pic.px, vec![1, 2, 2, 2]);

        // Draw on the tile: the level changes with it.
        p.layer_mut(art).unwrap().pixels_mut().unwrap().set(IVec2::new(1, 1), 1);
        let pic = Renderer::new(&p).indexed(IRect::new(10, 0, 2, 2), &[level]);
        assert_eq!(pic.px, vec![1, 2, 2, 1]);
    }

    #[test]
    fn instances_recolour_through_swap_slots() {
        let (mut p, art) = project();
        p.sprites.push(Sprite::new("t", vec![art], vec![Frame::new(IRect::new(0, 0, 2, 2))]));
        p.swaps.push(crate::project::SwapSlot {
            name: "shirt".into(),
            index: 2,
            options: "S".into(),
        });
        p.add_layer(
            "actors",
            LayerKind::Instances(vec![Instance {
                sprite: "t".into(),
                frame: 0,
                pos: IVec2::new(4, 0),
                transform: D4::IDENTITY,
                swaps: vec![("shirt".into(), 1)],
            }]),
        );
        let actors = p.layers[1].id;
        let pic = Renderer::new(&p).indexed(IRect::new(4, 0, 2, 2), &[actors]);
        assert_eq!(pic.px, vec![1, 1, 1, 1]);
    }

    #[test]
    fn translucent_entries_blend() {
        let (p, _) = project();
        let img = Renderer::new(&p).rgba(IRect::new(1, 1, 1, 1), [255, 255, 255, 255]);
        assert_eq!(img.px, vec![127, 127, 255, 255]);
    }
}
