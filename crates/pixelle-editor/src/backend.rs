//! Turning the project into pictures on screen.
//!
//! The editor only talks to [`CanvasBackend`], so how the canvas becomes pixels can change without
//! touching anything else. [`CpuTiles`] is the one there is today: it composites the canvas on
//! the CPU in 64×64 tiles with [`pixelle_core::render`], uploads each as a texture, and redraws a
//! tile only when something in it changed. A GPU backend would keep chunks and the palette on the
//! GPU and composite in a shader; the trait is shaped so that it can.

use std::collections::HashMap;

use eframe::egui::{self, Color32, ColorImage, Painter, Rect, TextureHandle, TextureId, TextureOptions, pos2};
use pixelle_core::render::{Indexed, Renderer, RgbaImage};
use pixelle_core::{IRect, Project};

use crate::doc::Changes;
use crate::view::View;

pub trait CanvasBackend {
    /// Forget whatever these changes made stale.
    fn invalidate(&mut self, changes: &Changes);

    /// Draw the visible part of the canvas. Returns whether some of it is still being prepared
    /// (the caller should repaint soon).
    fn paint(&mut self, ctx: &egui::Context, painter: &Painter, project: &Project, view: &View, screen: Rect) -> bool;

    /// One exported frame of a sprite, as a texture, with its size in pixels.
    fn frame(&mut self, ctx: &egui::Context, project: &Project, sprite: &str, frame: usize) -> Option<(TextureId, [u32; 2])>;

    /// A texture of an arbitrary indexed picture, for overlays such as a floating selection.
    /// `key` identifies the picture: the same key is not re-uploaded.
    fn overlay(&mut self, ctx: &egui::Context, key: u64, image: &Indexed, project: &Project) -> TextureId;

    /// A small picture of everything in `bounds`, at most `max` pixels on a side.
    fn thumbnail(&mut self, ctx: &egui::Context, project: &Project, revision: u64, bounds: IRect, max: u32) -> Option<TextureId>;
}

type Sized = (TextureHandle, [u32; 2]);

const TILE: i32 = 64;
/// Tiles composited per frame, so a big jump never stalls the UI.
const BUDGET: usize = 24;
/// Tiles kept before the far ones are forgotten.
const KEEP: usize = 2048;

#[derive(Default)]
pub struct CpuTiles {
    /// `None`: rendered, and empty.
    tiles: HashMap<(i32, i32), Option<TextureHandle>>,
    /// `(sprite, frame)` → the frame's texture and size, `None` if it is empty.
    frames: HashMap<(String, usize), Option<Sized>>,
    overlays: HashMap<u64, TextureHandle>,
    thumb: Option<(u64, IRect, TextureHandle)>,
}

fn upload(ctx: &egui::Context, name: &str, img: &RgbaImage) -> TextureHandle {
    let image = ColorImage::from_rgba_unmultiplied([img.w as usize, img.h as usize], &img.px);
    ctx.load_texture(name, image, TextureOptions::NEAREST)
}

impl CanvasBackend for CpuTiles {
    fn invalidate(&mut self, changes: &Changes) {
        if changes.all {
            self.tiles.clear();
            self.frames.clear();
            self.overlays.clear();
            return;
        }
        for r in &changes.rects {
            let (x0, y0) = (r.x.div_euclid(TILE), r.y.div_euclid(TILE));
            let (x1, y1) = ((r.max().x - 1).div_euclid(TILE), (r.max().y - 1).div_euclid(TILE));
            for ty in y0..=y1 {
                for tx in x0..=x1 {
                    self.tiles.remove(&(tx, ty));
                }
            }
        }
        if !changes.rects.is_empty() {
            self.frames.clear();
        }
    }

    fn paint(&mut self, ctx: &egui::Context, painter: &Painter, project: &Project, view: &View, screen: Rect) -> bool {
        let vis = view.visible(screen);
        let (x0, y0) = (vis.x.div_euclid(TILE), vis.y.div_euclid(TILE));
        let (x1, y1) = ((vis.max().x - 1).div_euclid(TILE), (vis.max().y - 1).div_euclid(TILE));

        // Render the missing tiles nearest the middle first.
        let mid = ((x0 + x1) / 2, (y0 + y1) / 2);
        let mut missing: Vec<(i32, i32)> = (y0..=y1)
            .flat_map(|y| (x0..=x1).map(move |x| (x, y)))
            .filter(|k| !self.tiles.contains_key(k))
            .collect();
        missing.sort_by_key(|(x, y)| (x - mid.0).abs() + (y - mid.1).abs());
        let pending = missing.len() > BUDGET;
        if !missing.is_empty() {
            let mut renderer = Renderer::new(project);
            for (tx, ty) in missing.into_iter().take(BUDGET) {
                let rect = IRect::new(tx * TILE, ty * TILE, TILE as u32, TILE as u32);
                let img = renderer.rgba(rect, [0; 4]);
                let tex = img
                    .px
                    .chunks_exact(4)
                    .any(|p| p[3] != 0)
                    .then(|| upload(ctx, &format!("tile {tx},{ty}"), &img));
                self.tiles.insert((tx, ty), tex);
            }
        }

        for ty in y0..=y1 {
            for tx in x0..=x1 {
                if let Some(Some(tex)) = self.tiles.get(&(tx, ty)) {
                    let r = view.rect_to_screen(screen, IRect::new(tx * TILE, ty * TILE, TILE as u32, TILE as u32));
                    painter.image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
            }
        }

        if self.tiles.len() > KEEP {
            self.tiles.retain(|(x, y), _| {
                *x >= x0 - 8 && *x <= x1 + 8 && *y >= y0 - 8 && *y <= y1 + 8
            });
        }
        pending
    }

    fn frame(&mut self, ctx: &egui::Context, project: &Project, sprite: &str, frame: usize) -> Option<(TextureId, [u32; 2])> {
        let entry = self
            .frames
            .entry((sprite.to_string(), frame))
            .or_insert_with(|| {
                let pic = Renderer::new(project).frame(sprite, frame)?;
                if pic.w == 0 || pic.h == 0 {
                    return None;
                }
                let tex = upload(ctx, &format!("{sprite}#{frame}"), &pic.to_rgba(&project.palette));
                Some((tex, [pic.w, pic.h]))
            });
        entry.as_ref().map(|(t, s)| (t.id(), *s))
    }

    fn overlay(&mut self, ctx: &egui::Context, key: u64, image: &Indexed, project: &Project) -> TextureId {
        if self.overlays.len() > 16 {
            self.overlays.clear();
        }
        self.overlays
            .entry(key)
            .or_insert_with(|| upload(ctx, "overlay", &image.to_rgba(&project.palette)))
            .id()
    }

    fn thumbnail(&mut self, ctx: &egui::Context, project: &Project, revision: u64, bounds: IRect, max: u32) -> Option<TextureId> {
        if bounds.is_empty() || bounds.w as u64 * bounds.h as u64 > 4_000_000 {
            return None;
        }
        if let Some((rev, b, tex)) = &self.thumb
            && *rev == revision
            && *b == bounds
        {
            return Some(tex.id());
        }
        let full = Renderer::new(project).rgba(bounds, [0; 4]);
        // Nearest-neighbour down to fit.
        let k = (bounds.w.max(bounds.h) as f32 / max as f32).max(1.0);
        let (w, h) = (((bounds.w as f32 / k) as u32).max(1), ((bounds.h as f32 / k) as u32).max(1));
        let mut small = RgbaImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let (sx, sy) = (((x as f32 + 0.5) * k) as u32, ((y as f32 + 0.5) * k) as u32);
                let s = ((sy.min(full.h - 1) * full.w + sx.min(full.w - 1)) * 4) as usize;
                let d = ((y * w + x) * 4) as usize;
                small.px[d..d + 4].copy_from_slice(&full.px[s..s + 4]);
            }
        }
        let tex = upload(ctx, "thumbnail", &small);
        let id = tex.id();
        self.thumb = Some((revision, bounds, tex));
        Some(id)
    }
}
