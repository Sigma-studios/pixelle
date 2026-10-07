//! A whole project: the canvas and everything that turns it into game assets.

use serde::{Deserialize, Serialize};

use crate::geom::IRect;
use crate::layer::{Layer, LayerId, LayerKind};
use crate::palette::Palette;
use crate::sprite::Sprite;
use crate::tiles::{Terrain, Tileset};

/// A palette index the game may recolour, and the palette section it picks the new colour from.
///
/// This is how a character's shirt becomes a choice: the sprite is drawn with the `shirt` entry
/// wherever the shirt is, and the game swaps that one index for any entry of "Clothing".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SwapSlot {
    pub name: String,
    pub index: u8,
    /// The label of the palette section the replacement comes from.
    pub options: String,
}

/// A named place on the canvas to jump to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    pub name: String,
    pub rect: IRect,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    /// The grid the editor snaps sprites and tiles to, in pixels.
    pub grid: u32,
    pub palette: Palette,
    /// Bottom to top.
    pub layers: Vec<Layer>,
    pub sprites: Vec<Sprite>,
    pub tilesets: Vec<Tileset>,
    /// A terrain layer cell holding `n` is `terrains[n - 1]`.
    pub terrains: Vec<Terrain>,
    pub swaps: Vec<SwapSlot>,
    pub bookmarks: Vec<Bookmark>,
    next_layer: LayerId,
}

impl Project {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            grid: 8,
            palette: Palette::default(),
            layers: Vec::new(),
            sprites: Vec::new(),
            tilesets: Vec::new(),
            terrains: Vec::new(),
            swaps: Vec::new(),
            bookmarks: Vec::new(),
            next_layer: 1,
        }
    }

    /// Add a layer on top of the others.
    pub fn add_layer(&mut self, name: impl Into<String>, kind: LayerKind) -> LayerId {
        let id = self.next_layer;
        self.next_layer += 1;
        self.layers.push(Layer {
            id,
            name: name.into(),
            visible: true,
            locked: false,
            reference: false,
            kind,
        });
        id
    }

    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    pub fn layer_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    pub fn layer_named(&self, name: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.name == name)
    }

    pub fn sprite(&self, name: &str) -> Option<&Sprite> {
        self.sprites.iter().find(|s| s.name == name)
    }

    pub fn tileset(&self, name: &str) -> Option<&Tileset> {
        self.tilesets.iter().find(|t| t.name == name)
    }

    /// The value a terrain layer cell holds for this terrain.
    pub fn terrain_id(&self, name: &str) -> Option<u8> {
        self.terrains
            .iter()
            .position(|t| t.name == name)
            .map(|i| i as u8 + 1)
    }

    pub fn swap(&self, name: &str) -> Option<&SwapSlot> {
        self.swaps.iter().find(|s| s.name == name)
    }

    /// Add a colour at the end of the `section`-th palette section, shifting later indices up and
    /// remapping every pixel, swap and swap slot to match.
    pub fn insert_color(
        &mut self,
        section: usize,
        name: impl Into<String>,
        rgba: crate::palette::Rgba,
    ) -> Result<u8, crate::palette::PaletteFull> {
        let s = &self.palette.sections()[section];
        let at = (s.start + s.len) as u8;
        // Remap before inserting: an old index at or past the gap moves up.
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
        let replace = if replace_with > index { replace_with - 1 } else { replace_with };
        let replace = if replace_with == index { 0 } else { replace };
        self.remap_indices(|v| match v {
            v if v == index => replace,
            v if v > index => v - 1,
            v => v,
        });
        self.swaps.retain(|s| s.index != index);
        self.palette.remove(index);
    }

    /// How many pixels on pixel layers hold this index.
    pub fn color_usage(&self, index: u8) -> usize {
        self.layers
            .iter()
            .filter_map(|l| l.pixels())
            .map(|g| g.iter().filter(|(_, v)| *v == index).count())
            .sum()
    }

    fn remap_indices(&mut self, f: impl Fn(u8) -> u8) {
        for layer in &mut self.layers {
            match &mut layer.kind {
                LayerKind::Pixels(g) => g.map_values(&f),
                LayerKind::Instances(list) => {
                    for i in list {
                        for (_, v) in &mut i.swaps {
                            *v = f(*v);
                        }
                    }
                }
                LayerKind::Terrain(_) => {}
            }
        }
        for s in &mut self.swaps {
            s.index = f(s.index);
        }
    }

    /// Everything drawn on the canvas, instances and terrain included.
    pub fn bounds(&self) -> IRect {
        self.bounds_where(|_| true)
    }

    /// Everything drawn on the layers `keep` accepts.
    pub fn bounds_where(&self, keep: impl Fn(&Layer) -> bool) -> IRect {
        let mut out = IRect::default();
        for layer in self.layers.iter().filter(|l| keep(l)) {
            let r = match &layer.kind {
                LayerKind::Pixels(grid) => grid.bounds(),
                LayerKind::Terrain(t) => t.cells.bounds().map(|b| {
                    let s = t.cell_size as i32;
                    IRect::from_min_max(b.min() * s, b.max() * s)
                }),
                LayerKind::Instances(list) => list.iter().fold(None, |acc: Option<IRect>, i| {
                    let Some(frame) = self
                        .sprite(&i.sprite)
                        .and_then(|s| s.frames.get(i.frame as usize))
                    else {
                        return acc;
                    };
                    let (w, h) = i.transform.size(frame.size().0, frame.size().1);
                    let r = IRect::new(i.pos.x, i.pos.y, w, h);
                    Some(acc.map_or(r, |a| a.union(&r)))
                }),
            };
            if let Some(r) = r {
                out = out.union(&r);
            }
        }
        out
    }

    /// Problems that would make an export wrong. Empty is good.
    pub fn validate(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut names = std::collections::HashSet::new();
        for sprite in &self.sprites {
            if !names.insert(&sprite.name) {
                out.push(format!("sprite {:?} is defined twice", sprite.name));
            }
            if sprite.frames.is_empty() {
                out.push(format!("sprite {:?} has no frames", sprite.name));
            }
            for id in &sprite.layers {
                match self.layer(*id) {
                    None => out.push(format!("sprite {:?} uses missing layer {id}", sprite.name)),
                    Some(l) if l.reference => out.push(format!(
                        "sprite {:?} exports reference layer {:?}",
                        sprite.name, l.name
                    )),
                    _ => {}
                }
            }
            for (i, frame) in sprite.frames.iter().enumerate() {
                // Points may sit on the far edge too: some mark a corner between pixels.
                let (w, h) = (frame.rect.w as i32, frame.rect.h as i32);
                for (name, p) in &frame.points {
                    if p.x < 0 || p.y < 0 || p.x > w || p.y > h {
                        out.push(format!(
                            "sprite {:?} frame {i}: point {name:?} {p} is outside the frame",
                            sprite.name
                        ));
                    }
                }
            }
        }
        for terrain in &self.terrains {
            match self.tileset(&terrain.tileset) {
                None => out.push(format!(
                    "terrain {:?} uses missing tileset {:?}",
                    terrain.name, terrain.tileset
                )),
                Some(ts) => {
                    for rule in &terrain.rules {
                        if rule.tile >= ts.count {
                            out.push(format!(
                                "terrain {:?} rule {:?} uses tile {} of {}",
                                terrain.name, rule.name, rule.tile, ts.count
                            ));
                        }
                    }
                }
            }
        }
        for swap in &self.swaps {
            if self.palette.section(&swap.options).is_none() {
                out.push(format!(
                    "swap slot {:?} picks from missing section {:?}",
                    swap.name, swap.options
                ));
            }
        }
        for layer in &self.layers {
            if let LayerKind::Instances(list) = &layer.kind {
                for i in list {
                    if self.sprite(&i.sprite).is_none() {
                        out.push(format!(
                            "layer {:?} instances missing sprite {:?}",
                            layer.name, i.sprite
                        ));
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::IVec2;
    use crate::grid::Grid;

    #[test]
    fn inserting_and_removing_colours_keeps_pixels_pointing_at_them() {
        let mut p = Project::new("t");
        p.palette.begin_section("A");
        let a = p.palette.push("a", [1, 0, 0, 255]).unwrap();
        p.palette.begin_section("B");
        let b = p.palette.push("b", [2, 0, 0, 255]).unwrap();
        let mut g = Grid::new();
        g.set(IVec2::new(0, 0), a);
        g.set(IVec2::new(1, 0), b);
        let l = p.add_layer("art", LayerKind::Pixels(g));

        let new = p.insert_color(0, "a2", [3, 0, 0, 255]).unwrap();
        assert_eq!(new, 2);
        let px = |p: &Project, x| p.palette.rgba(p.layer(l).unwrap().pixels().unwrap().get(IVec2::new(x, 0)));
        assert_eq!(px(&p, 0), [1, 0, 0, 255]);
        assert_eq!(px(&p, 1), [2, 0, 0, 255]);
        assert_eq!(p.palette.section("B").unwrap().start, 3);

        p.remove_color(1, 2);
        assert_eq!(px(&p, 0), [3, 0, 0, 255]);
        assert_eq!(px(&p, 1), [2, 0, 0, 255]);
        assert_eq!(p.palette.section("A").unwrap().len, 1);
    }
}
