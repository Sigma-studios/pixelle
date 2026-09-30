//! Packing every sprite frame into one indexed atlas.

use std::collections::{BTreeMap, HashMap};

use crate::geom::IRect;
use crate::project::Project;
use crate::render::{Indexed, Renderer};

pub struct Atlas {
    pub width: u32,
    pub height: u32,
    /// Palette indices, row-major.
    pub pixels: Vec<u8>,
    /// Where each frame of each sprite landed, in sprite order. Frames with identical pixels
    /// share one place.
    pub sprites: BTreeMap<String, Vec<IRect>>,
}

impl Atlas {
    pub fn image(&self) -> Indexed {
        Indexed {
            w: self.width,
            h: self.height,
            px: self.pixels.clone(),
        }
    }
}

/// Pack every frame of every sprite, `gutter` pixels apart. Each frame's edge pixels are repeated
/// into its gutter, so a renderer sampling a hair outside a frame gets the frame's own edge rather
/// than its neighbour's.
///
/// Deterministic: the same project always packs the same way.
pub fn pack(project: &Project, gutter: u32) -> Atlas {
    let mut renderer = Renderer::new(project);
    let mut unique: Vec<Indexed> = Vec::new();
    let mut seen: HashMap<Indexed, usize> = HashMap::new();
    let mut refs: Vec<(String, Vec<usize>)> = Vec::new();
    for sprite in &project.sprites {
        let mut frames = Vec::new();
        for i in 0..sprite.frames.len() {
            let pic = renderer.frame(&sprite.name, i).expect("the sprite exists");
            let id = *seen.entry(pic.clone()).or_insert_with(|| {
                unique.push(pic);
                unique.len() - 1
            });
            frames.push(id);
        }
        refs.push((sprite.name.clone(), frames));
    }

    // Shelves, tallest first, in the narrowest power-of-two width that keeps the atlas no taller
    // than it is wide.
    let mut order: Vec<usize> = (0..unique.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(unique[i].h), std::cmp::Reverse(unique[i].w), i));
    let slot = |i: usize| (unique[i].w + 2 * gutter, unique[i].h + 2 * gutter);
    let widest = order.iter().map(|&i| slot(i).0).max().unwrap_or(1);
    let area: u32 = order.iter().map(|&i| slot(i).0 * slot(i).1).sum();
    let mut width = widest.max(1).next_power_of_two();
    let (placed, height) = loop {
        let (placed, height) = shelve(&order, slot, width);
        if height <= width || width >= 8192 || width * width >= area * 4 {
            break (placed, height);
        }
        width *= 2;
    };

    let mut out = Indexed::new(width, height.max(1));
    let mut at = vec![IRect::default(); unique.len()];
    for (&i, &(x, y)) in order.iter().zip(&placed) {
        let pic = &unique[i];
        let (ox, oy) = (x + gutter, y + gutter);
        let g = gutter as i32;
        for dy in -g..pic.h as i32 + g {
            for dx in -g..pic.w as i32 + g {
                let sx = dx.clamp(0, pic.w as i32 - 1) as u32;
                let sy = dy.clamp(0, pic.h as i32 - 1) as u32;
                out.set(
                    (ox as i32 + dx) as u32,
                    (oy as i32 + dy) as u32,
                    pic.get(sx, sy),
                );
            }
        }
        at[i] = IRect::new(ox as i32, oy as i32, pic.w, pic.h);
    }

    Atlas {
        width: out.w,
        height: out.h,
        pixels: out.px,
        sprites: refs
            .into_iter()
            .map(|(name, ids)| (name, ids.into_iter().map(|i| at[i]).collect()))
            .collect(),
    }
}

/// Place slots left to right in rows. Returns each slot's top-left, in `order`, and the height.
fn shelve(order: &[usize], slot: impl Fn(usize) -> (u32, u32), width: u32) -> (Vec<(u32, u32)>, u32) {
    let (mut x, mut y, mut row) = (0, 0, 0);
    let mut placed = Vec::with_capacity(order.len());
    for &i in order {
        let (w, h) = slot(i);
        if x + w > width {
            x = 0;
            y += row;
            row = 0;
        }
        placed.push((x, y));
        x += w;
        row = row.max(h);
    }
    (placed, y + row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::IVec2;
    use crate::grid::Grid;
    use crate::layer::LayerKind;
    use crate::sprite::{Frame, Sprite};

    #[test]
    fn packs_frames_with_extruded_gutters_and_shares_duplicates() {
        let mut p = Project::new("t");
        let mut g = Grid::new();
        g.fill_rect(IRect::new(0, 0, 3, 2), 1);
        g.set(IVec2::new(0, 0), 2);
        g.fill_rect(IRect::new(10, 0, 3, 2), 1);
        g.set(IVec2::new(10, 0), 2);
        let art = p.add_layer("art", LayerKind::Pixels(g));
        p.sprites.push(Sprite::new(
            "a",
            vec![art],
            vec![Frame::new(IRect::new(0, 0, 3, 2)), Frame::new(IRect::new(10, 0, 3, 2))],
        ));
        let atlas = pack(&p, 1);
        let rects = &atlas.sprites["a"];
        assert_eq!(rects[0], rects[1], "identical frames share a place");
        let img = atlas.image();
        let r = rects[0];
        assert_eq!(img.get(r.x as u32, r.y as u32), 2);
        // The gutter repeats the edge.
        assert_eq!(img.get(r.x as u32 - 1, r.y as u32 - 1), 2);
        let again = pack(&p, 1);
        assert_eq!(again.pixels, atlas.pixels);
    }
}
