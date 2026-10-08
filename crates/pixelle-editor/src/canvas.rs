//! The canvas: what the pointer does with each tool, and everything drawn over the art.

use std::collections::HashMap;

use eframe::egui::{
    self, Align2, Color32, FontId, Key, Painter, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2,
};
use pixelle_core::layer::LayerKind;
use pixelle_core::render::Renderer;
use pixelle_core::{IRect, IVec2, LayerId, Sprite};

use crate::app::{Drag, Editor, Floating, Selected};
use crate::tools::{self, Tool};
use crate::workspace::{self, TabKind};

const MINIMAP: egui::Vec2 = vec2(200.0, 130.0);
const HANDLE: f32 = 7.0;

impl Editor {
    pub fn canvas(&mut self, ui: &mut egui::Ui) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        self.canvas_rect = rect;
        if self.fit_pending {
            self.fit_pending = false;
            let r = self.pending_fit_rect.take().unwrap_or_else(|| self.doc.project.bounds_where(|l| l.visible));
            if r.is_empty() {
                self.view.center = egui::Vec2::ZERO;
                self.view.zoom = 8.0;
            } else {
                self.view.fit(rect, r);
            }
        }
        self.input(ui, &response, rect);

        let painter = ui.painter_at(rect);
        let ctx = ui.ctx().clone();
        self.pending_tiles = self.backend.paint(&ctx, &painter, &self.doc.project, &self.view, rect);
        if self.pending_tiles {
            ctx.request_repaint();
        }
        self.overlays(&ctx, &painter, rect, response.hover_pos());
    }

    // --- Input ---------------------------------------------------------------------------------

    fn input(&mut self, ui: &egui::Ui, response: &egui::Response, rect: Rect) {
        let ctx = ui.ctx();
        if response.hovered() {
            let (scroll, zoom) = ctx.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
            let anchor = response.hover_pos().unwrap_or(rect.center());
            if zoom != 1.0 {
                self.zoom_acc *= zoom;
                if self.zoom_acc > 1.15 {
                    self.view.step_zoom(rect, anchor, 1);
                    self.zoom_acc = 1.0;
                } else if self.zoom_acc < 0.87 {
                    self.view.step_zoom(rect, anchor, -1);
                    self.zoom_acc = 1.0;
                }
            } else if scroll != egui::Vec2::ZERO {
                self.view.pan_by_screen(scroll);
            }
        }

        let (pressed, released, delta, mods, space) = ctx.input(|i| {
            let pressed = [PointerButton::Primary, PointerButton::Secondary, PointerButton::Middle]
                .into_iter()
                .find(|b| i.pointer.button_pressed(*b));
            (pressed, i.pointer.any_released(), i.pointer.delta(), i.modifiers, i.key_down(Key::Space))
        });
        let space = space && !ctx.egui_wants_keyboard_input();
        let Some(pos) = response.interact_pointer_pos().or(response.hover_pos()).or(ctx.pointer_latest_pos()) else {
            return;
        };
        let px = self.view.pixel(rect, pos);

        if let Some(button) = pressed
            && response.hovered()
        {
            self.drag = if button == PointerButton::Middle || space || self.tool == Tool::Hand {
                Some(Drag::Pan)
            } else if self.opts.minimap && self.minimap_rect(rect).contains(pos) {
                self.minimap_jump(rect, pos);
                Some(Drag::Minimap)
            } else {
                self.press(px, pos, button, mods)
            };
        } else if let Some(drag) = self.drag.clone() {
            match drag {
                Drag::Pan => self.view.pan_by_screen(delta),
                Drag::Minimap => self.minimap_jump(rect, pos),
                d => self.drag_to(d, px, mods),
            }
        }
        if released && let Some(drag) = self.drag.take() {
            self.release(drag, px);
        }
    }

    fn press(&mut self, px: IVec2, pos: Pos2, button: PointerButton, mods: egui::Modifiers) -> Option<Drag> {
        let secondary = button == PointerButton::Secondary;

        if let Some(name) = self.placing_point.clone() {
            if let Selected::Sprite { name: sprite, frame } = self.selected.clone() {
                let rect = self.doc.project.sprite(&sprite).and_then(|s| s.frames.get(frame)).map(|f| f.rect);
                if let Some(r) = rect.filter(|r| r.contains(px)) {
                    let local = px - r.min();
                    self.edit("Place point", None, |p| {
                        if let Some(f) = sprite_mut(p, &sprite).and_then(|s| s.frames.get_mut(frame)) {
                            f.points.insert(name.clone(), local);
                        }
                    });
                    self.say(format!("Placed {name:?} at {}, {}", local.x, local.y));
                    self.placing_point = None;
                    return None;
                }
            }
            self.say("Click inside the selected frame to place the point");
            return None;
        }

        if self.tool.paints() && mods.alt {
            self.pick(px, secondary);
            return None;
        }

        match self.tool {
            Tool::Pencil | Tool::Eraser => {
                let layer = self.need_pixel_layer()?;
                let value = match (self.tool, secondary) {
                    (Tool::Eraser, _) => 0,
                    (_, true) => self.secondary,
                    _ => self.primary,
                };
                let mirror = self.opts.mirror.then(|| self.frame_at(px).map(|(.., r)| r)).flatten();
                self.doc.begin(layer, if value == 0 { "Erase" } else { "Pencil" });
                self.paint_px(px, value, mirror);
                Some(Drag::Paint { value, recent: vec![px], mirror })
            }
            Tool::Line | Tool::Rect => {
                self.need_pixel_layer()?;
                let value = if secondary { self.secondary } else { self.primary };
                Some(Drag::Shape { start: px, value })
            }
            Tool::Fill => {
                self.fill(px, secondary, mods.shift);
                None
            }
            Tool::Picker => {
                self.pick(px, secondary);
                None
            }
            Tool::Select => {
                let inside = self.selection.is_some_and(|s| s.contains(px));
                if inside && self.lift_selection() {
                    let at = self.floating.as_ref().map_or(px, |f| f.pos);
                    Some(Drag::MoveFloating { grab: px - at })
                } else {
                    self.commit_floating();
                    self.selection = Some(tools::span(px, px));
                    Some(Drag::Select { start: px })
                }
            }
            Tool::Terrain => {
                let layer = self.active_layer?;
                let Some(LayerKind::Terrain(t)) = self.doc.project.layer(layer).map(|l| &l.kind) else {
                    self.say("Pick a terrain layer to paint terrain on");
                    return None;
                };
                if self.doc.project.layer(layer).is_some_and(|l| l.locked) {
                    self.say("The layer is locked");
                    return None;
                }
                let cell = cell_of(px, t.cell_size);
                let value = if secondary { 0 } else { self.active_terrain };
                self.doc.begin(layer, "Terrain");
                self.doc.set(cell, value);
                Some(Drag::Terrain { value, last: cell })
            }
            Tool::Sprite => {
                if let Selected::Sprite { name, frame } = self.selected.clone()
                    && let Some(r) = self.doc.project.sprite(&name).and_then(|s| s.frames.get(frame)).map(|f| f.rect)
                {
                    let corner = self.view.to_screen(self.canvas_rect, pos2(r.max().x as f32, r.max().y as f32));
                    if (corner - pos).length() < HANDLE * 1.5 {
                        return Some(Drag::ResizeFrame { frame, orig: r });
                    }
                }
                if let Some((name, frame, _)) = self.frame_at(px) {
                    self.select_sprite(&name, frame);
                }
                None
            }
            Tool::Hand => Some(Drag::Pan),
        }
    }

    fn drag_to(&mut self, drag: Drag, px: IVec2, mods: egui::Modifiers) {
        match drag {
            Drag::Paint { value, mut recent, mirror } => {
                let last = *recent.last().expect("a paint drag starts with a pixel");
                if px == last {
                    return;
                }
                for p in tools::line(last, px).into_iter().skip(1) {
                    self.paint_px(p, value, mirror);
                    recent.push(p);
                    let n = recent.len();
                    if self.opts.pixel_perfect && n >= 3 && tools::is_elbow(recent[n - 3], recent[n - 2], recent[n - 1]) {
                        let elbow = recent.remove(n - 2);
                        self.doc.revert(elbow);
                        if let Some(m) = mirror {
                            self.doc.revert(mirrored(elbow, m));
                        }
                    }
                }
                let keep = recent.len().saturating_sub(3);
                recent.drain(..keep);
                self.drag = Some(Drag::Paint { value, recent, mirror });
            }
            Drag::Shape { start, value } => {
                self.drag = Some(Drag::Shape { start, value });
                self.shape_end = Some(px);
                self.shape_filled = mods.shift;
            }
            Drag::Select { start } => self.selection = Some(tools::span(start, px)),
            Drag::MoveFloating { grab } => {
                if let Some(f) = &mut self.floating {
                    f.pos = px - grab;
                    self.selection = Some(IRect::new(f.pos.x, f.pos.y, f.img.w, f.img.h));
                }
            }
            Drag::ResizeFrame { frame, orig } => {
                let mut end = px + IVec2::ONE;
                if !mods.alt {
                    end = self.snap(end + IVec2::splat(self.doc.project.grid as i32 / 2));
                }
                let (w, h) = ((end.x - orig.x).max(1) as u32, (end.y - orig.y).max(1) as u32);
                let now = self.doc.project.sprites.first().and_then(|s| s.frames.get(frame)).map(|f| (f.rect.w, f.rect.h));
                if now.is_some_and(|n| n != (w, h)) {
                    self.edit_sprite("Resize frame", Some("resize-frame"), |s| workspace::resize_frame(&mut s.frames[frame], w, h));
                }
                self.drag = Some(Drag::ResizeFrame { frame, orig });
            }
            Drag::Terrain { value, last } => {
                let size = self.active_layer.and_then(|l| match &self.doc.project.layer(l)?.kind {
                    LayerKind::Terrain(t) => Some(t.cell_size),
                    _ => None,
                });
                let Some(size) = size else { return };
                let cell = cell_of(px, size);
                for c in tools::line(last, cell) {
                    self.doc.set(c, value);
                }
                self.drag = Some(Drag::Terrain { value, last: cell });
            }
            Drag::Pan | Drag::Minimap => {}
        }
    }

    fn release(&mut self, drag: Drag, px: IVec2) {
        match drag {
            Drag::Paint { .. } | Drag::Terrain { .. } => self.doc.end(),
            Drag::Shape { start, value } => {
                let end = self.shape_end.take().unwrap_or(px);
                let Some(layer) = self.pixel_layer() else { return };
                let filled = self.shape_filled;
                let pixels = match self.tool {
                    Tool::Line => tools::line(start, end),
                    _ => tools::rect_pixels(tools::span(start, end), filled),
                };
                self.doc.begin(layer, self.tool.name());
                for p in pixels {
                    self.doc.set(p, value);
                }
                self.doc.end();
            }
            Drag::Select { start }
                if start == px && self.selection.is_some_and(|s| s.w == 1 && s.h == 1) => {
                    self.selection = None;
                }
            _ => {}
        }
        self.shape_end = None;
    }

    // --- Tool helpers --------------------------------------------------------------------------

    fn need_pixel_layer(&mut self) -> Option<LayerId> {
        let l = self.pixel_layer();
        if l.is_none() {
            self.say("Pick an unlocked pixel layer to draw on");
        }
        l
    }

    fn paint_px(&mut self, p: IVec2, value: u8, mirror: Option<IRect>) {
        self.doc.set(p, value);
        if let Some(m) = mirror {
            self.doc.set(mirrored(p, m), value);
        }
    }

    pub fn snap(&self, p: IVec2) -> IVec2 {
        let g = self.doc.project.grid.max(1) as i32;
        IVec2::new(p.x.div_euclid(g) * g, p.y.div_euclid(g) * g)
    }

    fn pick(&mut self, px: IVec2, secondary: bool) {
        let visible: Vec<LayerId> = self.doc.project.layers.iter().filter(|l| l.visible).map(|l| l.id).collect();
        let v = Renderer::new(&self.doc.project).indexed(IRect::new(px.x, px.y, 1, 1), &visible).px[0];
        if secondary {
            self.secondary = v;
        } else {
            self.primary = v;
        }
    }

    /// Flood the region under `px` on the active layer.
    ///
    /// A region of a colour is finite, so it always fills whole. Transparent space is the one
    /// kind that can go on forever: it fills when something encloses it, and otherwise nothing
    /// happens and the status bar says so. Inside a selection, the selection is the boundary,
    /// which is how to fill open space on purpose.
    fn fill(&mut self, px: IVec2, secondary: bool, dither: bool) {
        self.commit_floating();
        let Some(layer) = self.need_pixel_layer() else { return };
        let grid = self.doc.grid(layer);
        let get = |p: IVec2| grid.map_or(0, |g| g.get(p));
        let region = match self.selection.filter(|s| s.contains(px)) {
            Some(sel) => tools::flood(px, sel, get),
            None => {
                // Everything drawn on the layer, one pixel wider: a region that reaches that
                // margin is open to the rest of the canvas.
                let content = grid.and_then(|g| g.bounds()).unwrap_or_default();
                let bounds = content.union(&IRect::new(px.x, px.y, 1, 1)).grow(1);
                let region = tools::flood(px, bounds, get);
                let edge = |p: &IVec2| p.x == bounds.x || p.y == bounds.y || p.x == bounds.max().x - 1 || p.y == bounds.max().y - 1;
                if get(px) == 0 && region.iter().any(edge) {
                    self.say("That space is open to the rest of the canvas: close it off, or select an area to fill");
                    return;
                }
                region
            }
        };
        let (a, b) = if secondary { (self.secondary, self.primary) } else { (self.primary, self.secondary) };
        self.doc.begin(layer, "Fill");
        for p in region {
            let v = if dither && (p.x + p.y).rem_euclid(2) == 1 { b } else { a };
            self.doc.set(p, v);
        }
        self.doc.end();
    }

    /// The topmost sprite frame under a pixel.
    pub fn frame_at(&self, px: IVec2) -> Option<(String, usize, IRect)> {
        self.doc.project.sprites.iter().rev().filter(|s| !workspace::is_anim(s)).find_map(|s| {
            s.frames.iter().enumerate().find(|(_, f)| f.rect.contains(px)).map(|(i, f)| (s.name.clone(), i, f.rect))
        })
    }

    pub fn select_sprite(&mut self, name: &str, frame: usize) {
        self.selected = Selected::Sprite { name: name.to_string(), frame };
    }

    // --- Minimap -------------------------------------------------------------------------------

    fn minimap_rect(&self, screen: Rect) -> Rect {
        Rect::from_min_size(screen.max - MINIMAP - vec2(10.0, 10.0), MINIMAP)
    }

    /// Where the minimap draws the project: its bounds scaled to fit, centred.
    fn minimap_mapping(&self, screen: Rect) -> Option<(IRect, Rect)> {
        let b = self.doc.project.bounds_where(|l| l.visible);
        if b.is_empty() {
            return None;
        }
        let area = self.minimap_rect(screen).shrink(6.0);
        let k = (area.width() / b.w as f32).min(area.height() / b.h as f32);
        let size = vec2(b.w as f32 * k, b.h as f32 * k);
        Some((b, Rect::from_center_size(area.center(), size)))
    }

    fn minimap_jump(&mut self, screen: Rect, pos: Pos2) {
        if let Some((b, r)) = self.minimap_mapping(screen) {
            let t = (pos - r.min) / r.size();
            self.view.center = vec2(b.x as f32 + t.x * b.w as f32, b.y as f32 + t.y * b.h as f32);
        }
    }

    // --- Overlays ------------------------------------------------------------------------------

    fn overlays(&mut self, ctx: &egui::Context, painter: &Painter, screen: Rect, hover: Option<Pos2>) {
        let vis = self.view.visible(screen);
        let zoom = self.view.zoom;
        let view = self.view;
        let to = move |p: IVec2| view.to_screen(screen, pos2(p.x as f32, p.y as f32));

        // Grids.
        let grid = self.doc.project.grid.max(1) as i32;
        let lines = |step: i32, color: Color32| {
            let x0 = vis.x.div_euclid(step) * step;
            let y0 = vis.y.div_euclid(step) * step;
            for x in (x0..=vis.max().x).step_by(step as usize) {
                let a = to(IVec2::new(x, vis.y));
                painter.line_segment([pos2(a.x, screen.top()), pos2(a.x, screen.bottom())], Stroke::new(1.0, color));
            }
            for y in (y0..=vis.max().y).step_by(step as usize) {
                let a = to(IVec2::new(vis.x, y));
                painter.line_segment([pos2(screen.left(), a.y), pos2(screen.right(), a.y)], Stroke::new(1.0, color));
            }
        };
        if self.opts.pixel_grid && zoom >= 8.0 {
            lines(1, Color32::from_white_alpha(10));
        }
        if self.opts.tile_grid && zoom * grid as f32 >= 24.0 {
            lines(grid, Color32::from_white_alpha(24));
        }

        // Highlighted colour.
        if let Some(h) = self.highlight
            && (vis.w as u64 * vis.h as u64) < 400_000
        {
            let t = self.time();
            let a = (128.0 + 100.0 * (t * 6.0).sin()) as u8;
            for l in self.doc.project.layers.iter().filter(|l| l.visible) {
                if let Some(g) = l.pixels() {
                    for (p, v) in g.iter() {
                        if v == h && vis.contains(p) {
                            painter.rect_filled(self.view.rect_to_screen(screen, IRect::new(p.x, p.y, 1, 1)), 0.0, Color32::from_rgba_unmultiplied(255, 0, 255, a));
                        }
                    }
                }
            }
            ctx.request_repaint();
        }

        // A terrain's tiles, by name.
        if let TabKind::Terrain(name) = self.kind().clone()
            && let Some(t) = self.project.terrains.get(&name)
            && let Some(ts) = self.doc.project.tilesets.first()
        {
            let slots = t.preset.two_surface().slots;
            let col = Color32::from_rgb(90, 200, 255);
            for (i, slot) in slots.iter().enumerate() {
                let r = self.view.rect_to_screen(screen, ts.tile_rect(i as u32));
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, col.gamma_multiply(0.6)), StrokeKind::Outside);
                if r.width() >= 40.0 {
                    painter.text(r.left_top() - vec2(0.0, 2.0), Align2::LEFT_BOTTOM, &slot.name, FontId::proportional(10.0), col);
                }
            }
            if let Some(patch) = workspace::patch_rect(&self.doc.project) {
                let s = ts.tile_size as i32;
                let px = IRect::new(patch.x * s, patch.y * s, patch.w * s as u32, patch.h * s as u32);
                let r = self.view.rect_to_screen(screen, px);
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_white_alpha(40)), StrokeKind::Outside);
                painter.text(r.left_top() - vec2(0.0, 2.0), Align2::LEFT_BOTTOM, "ground to try them on (T paints, right button erases)", FontId::proportional(11.0), Color32::from_white_alpha(140));
            }
        }

        // Sprites, their animation previews and anchors.
        self.sprite_overlays(ctx, painter, screen, vis, hover);

        // Floating pixels and the selection.
        if let Some(f) = &self.floating {
            let Floating { img, pos, key, .. } = f;
            let tex = self.backend.overlay(ctx, *key, img, &self.doc.project);
            let r = self.view.rect_to_screen(screen, IRect::new(pos.x, pos.y, img.w, img.h));
            painter.image(tex, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        if let Some(sel) = self.selection {
            let r = self.view.rect_to_screen(screen, sel);
            let t = self.time();
            let phase = ((t * 4.0) as i32 % 2) == 0;
            let (a, b) = if phase { (Color32::WHITE, Color32::BLACK) } else { (Color32::BLACK, Color32::WHITE) };
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, a), StrokeKind::Outside);
            painter.rect_stroke(r.expand(1.0), 0.0, Stroke::new(1.0, b), StrokeKind::Outside);
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }

        // Shape previews.
        let primary = self.doc.project.palette.rgba(self.primary);
        let paint_color = |v: u8| {
            let c = self.doc.project.palette.rgba(v);
            if c[3] == 0 { Color32::from_rgba_unmultiplied(255, 60, 60, 160) } else { Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]) }
        };
        match (&self.drag, self.shape_end) {
            (Some(Drag::Shape { start, value }), Some(end)) => {
                let pixels = if self.tool == Tool::Line {
                    tools::line(*start, end)
                } else {
                    tools::rect_pixels(tools::span(*start, end), self.shape_filled)
                };
                let c = paint_color(*value);
                for p in pixels {
                    painter.rect_filled(self.view.rect_to_screen(screen, IRect::new(p.x, p.y, 1, 1)), 0.0, c);
                }
            }
            _ => {}
        }

        // The cursor.
        if let Some(h) = hover.filter(|h| screen.contains(*h))
            && self.drag.as_ref().is_none_or(|d| matches!(d, Drag::Paint { .. } | Drag::Terrain { .. }))
            && !(self.opts.minimap && self.minimap_rect(screen).contains(h))
        {
            let px = self.view.pixel(screen, h);
            let cell = match (self.tool, self.active_layer.and_then(|l| self.doc.project.layer(l)).map(|l| &l.kind)) {
                (Tool::Terrain, Some(LayerKind::Terrain(t))) => {
                    let s = t.cell_size as i32;
                    let c = cell_of(px, t.cell_size);
                    IRect::new(c.x * s, c.y * s, t.cell_size, t.cell_size)
                }
                _ => IRect::new(px.x, px.y, 1, 1),
            };
            let r = self.view.rect_to_screen(screen, cell);
            if matches!(self.tool, Tool::Pencil | Tool::Line | Tool::Rect | Tool::Fill) && primary[3] > 0 {
                painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(primary[0], primary[1], primary[2], 200));
            }
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_white_alpha(180)), StrokeKind::Outside);
        }

        if self.opts.minimap {
            self.minimap(ctx, painter, screen);
        }
    }

    fn sprite_overlays(&mut self, ctx: &egui::Context, painter: &Painter, screen: Rect, vis: IRect, hover: Option<Pos2>) {
        let hovered = hover.filter(|h| screen.contains(*h)).and_then(|h| self.frame_at(self.view.pixel(screen, h))).map(|(n, ..)| n);
        let selected = match &self.selected {
            Selected::Sprite { name, frame } => Some((name.clone(), *frame)),
            _ => None,
        };
        let zoom = self.view.zoom;
        let label_font = FontId::proportional(11.0);

        if self.opts.previews {
            self.refresh_previews();
        }
        let t = self.time();
        let mut animating = false;
        let sprites = self.doc.project.sprites.clone();
        for s in &sprites {
            let anim = workspace::is_anim(s);
            let is_sel = selected.as_ref().is_some_and(|(n, _)| *n == s.name);
            let is_hover = hovered.as_deref() == Some(s.name.as_str());
            let visible = s.frames.iter().any(|f| !f.rect.intersect(&vis).is_empty());

            if self.opts.sprite_boxes && visible && !anim {
                for (i, f) in s.frames.iter().enumerate() {
                    let r = self.view.rect_to_screen(screen, f.rect);
                    let (w, c) = if is_sel {
                        let current = selected.as_ref().is_some_and(|(_, fr)| *fr == i);
                        (if current { 2.0 } else { 1.0 }, Color32::from_rgb(255, 210, 60))
                    } else if is_hover {
                        (1.0, Color32::from_white_alpha(160))
                    } else {
                        (1.0, Color32::from_white_alpha(28))
                    };
                    painter.rect_stroke(r, 0.0, Stroke::new(w, c), StrokeKind::Outside);
                    if is_sel && selected.as_ref().is_some_and(|(_, fr)| *fr == i) && self.tool == Tool::Sprite {
                        painter.rect_filled(Rect::from_center_size(r.max, vec2(HANDLE, HANDLE)), 1.0, Color32::from_rgb(255, 210, 60));
                    }
                }
                if is_sel || is_hover {
                    let r = self.view.rect_to_screen(screen, s.frames[0].rect);
                    let text = if s.is_animation() { format!("{} ▶", s.name) } else { s.name.clone() };
                    let galley_rect = painter.text(r.left_top() - vec2(0.0, 3.0), Align2::LEFT_BOTTOM, &text, label_font.clone(), Color32::WHITE);
                    painter.rect_filled(galley_rect.expand(2.0), 2.0, Color32::from_black_alpha(160));
                    painter.text(r.left_top() - vec2(0.0, 3.0), Align2::LEFT_BOTTOM, &text, label_font.clone(), Color32::WHITE);
                }
            }

            // Anchors of the selected sprite.
            if is_sel && !anim {
                for f in &s.frames {
                    for (name, p) in &f.points {
                        let c = self.view.to_screen(screen, pos2(f.rect.x as f32 + p.x as f32 + 0.5, f.rect.y as f32 + p.y as f32 + 0.5));
                        let k = (zoom * 0.5).clamp(3.0, 8.0);
                        let col = Color32::from_rgb(255, 80, 200);
                        painter.line_segment([c - vec2(k, 0.0), c + vec2(k, 0.0)], Stroke::new(1.5, col));
                        painter.line_segment([c - vec2(0.0, k), c + vec2(0.0, k)], Stroke::new(1.5, col));
                        if zoom >= 12.0 {
                            painter.text(c + vec2(k, -k), Align2::LEFT_BOTTOM, name, FontId::proportional(10.0), col);
                        }
                    }
                    for r in f.rects.values() {
                        let rr = self.view.rect_to_screen(screen, r.offset(f.rect.min()));
                        painter.rect_stroke(rr, 0.0, Stroke::new(1.0, Color32::from_rgb(80, 220, 255)), StrokeKind::Inside);
                    }
                }
            }

            // Animation preview, playing beside the strip.
            if self.opts.previews
                && s.is_animation()
                && let Some(at) = self.previews.1.get(&s.name).copied()
                && !at.intersect(&vis).is_empty()
            {
                let total: f32 = s.frames.iter().map(|f| f.duration.max(0.001)).sum();
                let mut clock = t % total;
                let mut frame = 0;
                for (i, f) in s.frames.iter().enumerate() {
                    if clock < f.duration.max(0.001) {
                        frame = i;
                        break;
                    }
                    clock -= f.duration.max(0.001);
                }
                let r = self.view.rect_to_screen(screen, at);
                painter.rect_filled(r, 0.0, Color32::from_black_alpha(90));
                if let Some((tex, _)) = self.backend.frame(ctx, &self.doc.project, &s.name, frame) {
                    painter.image(tex, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_rgb(120, 200, 255)), StrokeKind::Outside);
                if let Some((_, name)) = s.name.rsplit_once(pixelle_core::folder::ANIM_SEPARATOR) {
                    painter.text(r.left_top() - vec2(0.0, 2.0), Align2::LEFT_BOTTOM, name, label_font.clone(), Color32::from_rgb(120, 200, 255));
                }
                animating = true;
            }
        }
        if animating {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    /// Work out, once per change, where each animation's preview goes: to the left of its first
    /// frame if that spot is empty, else above it.
    fn refresh_previews(&mut self) {
        if self.previews.0 == self.doc.revision {
            return;
        }
        let p = &self.doc.project;
        let all: Vec<LayerId> = p.layers.iter().filter(|l| !matches!(l.kind, LayerKind::Instances(_))).map(|l| l.id).collect();
        let mut renderer = Renderer::new(p);
        let mut taken: Vec<IRect> = p.sprites.iter().flat_map(|s| s.frames.iter().map(|f| f.rect)).collect();
        let mut out = HashMap::new();
        // A sheet's animations play in a column to its left, one under the other.
        let mut column: HashMap<String, i32> = HashMap::new();
        for s in p.sprites.iter().filter(|s| workspace::is_anim(s) && s.is_animation()) {
            let sheet = s.tags.get(pixelle_core::folder::ANIM_OF).and_then(|n| p.sprite(n));
            let Some(sheet) = sheet else { continue };
            let left = sheet.frames.iter().map(|f| f.rect.x).min().unwrap_or(0);
            let w = s.frames.iter().map(|f| f.size().0).max().unwrap_or(1);
            let h = s.frames.iter().map(|f| f.size().1).max().unwrap_or(1);
            let gap = (w.min(h) as i32 / 2).max(4);
            let y = column.entry(sheet.name.clone()).or_insert(sheet.frames[0].rect.y);
            let r = IRect::new(left - 2 * gap - w as i32, *y, w, h);
            *y += h as i32 + 2 * gap;
            taken.push(r);
            out.insert(s.name.clone(), r);
        }
        for s in p.sprites.iter().filter(|s| s.is_animation() && !workspace::is_anim(s)) {
            let f0 = &s.frames[0];
            let (w, h) = f0.size();
            let gap = (w.min(h) as i32 / 4).max(2);
            let candidates = [
                IRect::new(f0.rect.x - w as i32 - gap, f0.rect.y, w, h),
                IRect::new(f0.rect.x, f0.rect.y - h as i32 - gap, w, h),
                IRect::new(f0.rect.x, f0.rect.max().y + gap, w, h),
            ];
            let free = |r: &IRect, taken: &[IRect], renderer: &mut Renderer| {
                taken.iter().all(|t| t.intersect(r).is_empty()) && renderer.indexed(*r, &all).is_blank()
            };
            if let Some(r) = candidates.into_iter().find(|r| free(r, &taken, &mut renderer)) {
                taken.push(r);
                out.insert(s.name.clone(), r);
            }
        }
        self.previews = (self.doc.revision, out);
    }

    fn minimap(&mut self, ctx: &egui::Context, painter: &Painter, screen: Rect) {
        let frame = self.minimap_rect(screen);
        painter.rect_filled(frame, 4.0, Color32::from_black_alpha(170));
        let Some((b, r)) = self.minimap_mapping(screen) else { return };
        if let Some(tex) = self.backend.thumbnail(ctx, &self.doc.project, self.doc.revision, b, 256) {
            painter.image(tex, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        let vis = self.view.visible(screen);
        let k = r.width() / b.w as f32;
        let v = Rect::from_min_size(
            r.min + vec2((vis.x - b.x) as f32 * k, (vis.y - b.y) as f32 * k),
            vec2(vis.w as f32 * k, vis.h as f32 * k),
        )
        .intersect(frame);
        painter.rect_stroke(v, 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
    }
}

pub fn cell_of(px: IVec2, size: u32) -> IVec2 {
    let s = size.max(1) as i32;
    IVec2::new(px.x.div_euclid(s), px.y.div_euclid(s))
}

fn mirrored(p: IVec2, r: IRect) -> IVec2 {
    IVec2::new(r.x + r.max().x - 1 - p.x, p.y)
}

pub fn sprite_mut<'a>(p: &'a mut pixelle_core::Project, name: &str) -> Option<&'a mut Sprite> {
    p.sprites.iter_mut().find(|s| s.name == name)
}

/// The tools, driven the way the pointer drives them.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Args;
    use pixelle_core::D4;
    use egui::Modifiers;

    fn editor() -> Editor {
        let mut ed = Editor::new(Args { project: None, remember: false, screenshot: None, open: None, zoom: None, tool: None, layer: None }).unwrap();
        ed.canvas_rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        ed.view.center = vec2(0.0, 0.0);
        ed.view.zoom = 4.0;
        ed
    }

    /// Press at the first point, drag through the rest, release at the last.
    fn drag(ed: &mut Editor, points: &[(i32, i32)], button: PointerButton, mods: Modifiers) {
        let px = |i: usize| IVec2::new(points[i].0, points[i].1);
        ed.drag = ed.press(px(0), Pos2::ZERO, button, mods);
        for i in 1..points.len() {
            if let Some(d) = ed.drag.clone() {
                ed.drag_to(d, px(i), mods);
            }
        }
        if let Some(d) = ed.drag.take() {
            ed.release(d, px(points.len() - 1));
        }
    }

    fn click(ed: &mut Editor, x: i32, y: i32) {
        drag(ed, &[(x, y)], PointerButton::Primary, Modifiers::NONE);
    }

    fn art(ed: &Editor) -> LayerId {
        ed.pixel_layer().unwrap()
    }

    #[test]
    fn pencil_strokes_are_pixel_perfect_and_undo_as_one() {
        let mut ed = editor();
        ed.tool = Tool::Pencil;
        ed.primary = 5;
        drag(&mut ed, &[(0, 0), (1, 0), (1, 1), (2, 1)], PointerButton::Primary, Modifiers::NONE);
        let l = art(&ed);
        // The elbow at (1, 0) is dropped so the diagonal stays one pixel thin.
        assert_eq!(ed.doc.get(l, IVec2::new(0, 0)), 5);
        assert_eq!(ed.doc.get(l, IVec2::new(1, 0)), 0);
        assert_eq!(ed.doc.get(l, IVec2::new(1, 1)), 5);
        ed.undo();
        assert_eq!(ed.doc.get(l, IVec2::new(0, 0)), 0);
        assert_eq!(ed.doc.get(l, IVec2::new(1, 1)), 0);
    }

    #[test]
    fn right_button_paints_the_secondary_and_lines_are_one_step() {
        let mut ed = editor();
        ed.tool = Tool::Line;
        ed.secondary = 7;
        drag(&mut ed, &[(0, 0), (3, 0), (4, 2)], PointerButton::Secondary, Modifiers::NONE);
        let l = art(&ed);
        assert_eq!(ed.doc.get(l, IVec2::new(4, 2)), 7);
        assert_eq!(ed.doc.get(l, IVec2::new(3, 0)), 0, "only the final line is drawn");
        assert_eq!(ed.doc.can_undo(), Some("Line"));
    }

    #[test]
    fn selections_lift_move_and_put_down() {
        let mut ed = editor();
        ed.tool = Tool::Rect;
        ed.primary = 3;
        drag(&mut ed, &[(0, 0), (1, 1)], PointerButton::Primary, Modifiers::SHIFT);
        ed.tool = Tool::Select;
        drag(&mut ed, &[(0, 0), (1, 1)], PointerButton::Primary, Modifiers::NONE);
        assert_eq!(ed.selection, Some(IRect::new(0, 0, 2, 2)));
        drag(&mut ed, &[(0, 0), (5, 4)], PointerButton::Primary, Modifiers::NONE);
        assert!(ed.floating.is_some());
        ed.commit_floating();
        let l = art(&ed);
        assert_eq!(ed.doc.get(l, IVec2::new(0, 0)), 0);
        assert_eq!(ed.doc.get(l, IVec2::new(5, 4)), 3);
        assert_eq!(ed.doc.get(l, IVec2::new(6, 5)), 3);
        ed.undo();
        assert_eq!(ed.doc.get(l, IVec2::new(0, 0)), 3);
        assert_eq!(ed.doc.get(l, IVec2::new(5, 4)), 0);
    }

    #[test]
    fn copy_paste_and_flip() {
        let mut ed = editor();
        let l = art(&ed);
        ed.tool = Tool::Pencil;
        ed.primary = 2;
        click(&mut ed, 0, 0);
        ed.selection = Some(IRect::new(0, 0, 2, 1));
        ed.copy(false);
        ed.paste(IVec2::new(10, 0));
        ed.transform_floating(D4::FLIP_X);
        ed.commit_floating();
        assert_eq!(ed.doc.get(l, IVec2::new(11, 0)), 2);
        assert_eq!(ed.doc.get(l, IVec2::new(10, 0)), 0);
    }

    #[test]
    fn fill_fills_enclosed_space_and_whole_regions() {
        let mut ed = editor();
        let l = art(&ed);
        // A 6x6 ring, open nowhere.
        ed.tool = Tool::Rect;
        ed.primary = 2;
        drag(&mut ed, &[(0, 0), (5, 5)], PointerButton::Primary, Modifiers::NONE);
        ed.tool = Tool::Fill;
        ed.primary = 9;
        click(&mut ed, 2, 2);
        assert_eq!(ed.doc.get(l, IVec2::new(1, 1)), 9);
        assert_eq!(ed.doc.get(l, IVec2::new(4, 4)), 9);
        assert_eq!(ed.doc.get(l, IVec2::new(6, 6)), 0, "outside stays empty");
        // The ring itself is a region of a colour: all of it goes, however far it runs.
        ed.primary = 5;
        click(&mut ed, 0, 3);
        assert_eq!(ed.doc.get(l, IVec2::new(5, 0)), 5);
        assert_eq!(ed.doc.get(l, IVec2::new(2, 2)), 9);
        // Shift dithers with the secondary.
        ed.secondary = 4;
        ed.primary = 8;
        drag(&mut ed, &[(1, 1)], PointerButton::Primary, Modifiers::SHIFT);
        assert_eq!(ed.doc.get(l, IVec2::new(2, 2)), 8);
        assert_eq!(ed.doc.get(l, IVec2::new(1, 2)), 4);
    }

    #[test]
    fn fill_leaves_open_space_alone_unless_it_is_selected() {
        let mut ed = editor();
        let l = art(&ed);
        ed.tool = Tool::Line;
        ed.primary = 2;
        drag(&mut ed, &[(0, 0), (0, 5)], PointerButton::Primary, Modifiers::NONE);
        ed.tool = Tool::Fill;
        click(&mut ed, 3, 3);
        assert_eq!(ed.doc.get(l, IVec2::new(3, 3)), 0);
        assert!(ed.doc.can_undo() == Some("Line"));
        ed.selection = Some(IRect::new(1, 1, 3, 3));
        click(&mut ed, 2, 2);
        assert_eq!(ed.doc.get(l, IVec2::new(3, 3)), 2);
        assert_eq!(ed.doc.get(l, IVec2::new(4, 4)), 0);
    }

    fn sprite_tab(ed: &mut Editor) {
        ed.new_file("hero", pixelle_core::folder::File::Sprite(pixelle_core::SpriteFile::new(8, 8))).unwrap();
        assert_eq!(ed.kind(), &TabKind::Sprite("hero".into()));
    }

    #[test]
    fn a_sprite_paints_inside_its_frames_only() {
        let mut ed = editor();
        sprite_tab(&mut ed);
        ed.tool = Tool::Line;
        ed.primary = 3;
        drag(&mut ed, &[(0, 0), (20, 0)], PointerButton::Primary, Modifiers::NONE);
        let l = art(&ed);
        assert_eq!(ed.doc.get(l, IVec2::new(7, 0)), 3);
        assert_eq!(ed.doc.get(l, IVec2::new(8, 0)), 0, "outside the frame belongs to no file");
        ed.write_back_all();
        assert_eq!(ed.project.sprites["hero"].frames[0].pixels["art"].get(7, 0), 3);
    }

    #[test]
    fn frames_resize_from_the_corner_as_one_step() {
        let mut ed = editor();
        sprite_tab(&mut ed);
        ed.tool = Tool::Pencil;
        ed.primary = 2;
        click(&mut ed, 1, 1);
        ed.drag = Some(Drag::ResizeFrame { frame: 0, orig: IRect::new(0, 0, 8, 8) });
        for p in [IVec2::new(11, 11), IVec2::new(15, 7)] {
            let d = ed.drag.clone().unwrap();
            ed.drag_to(d, p, Modifiers::NONE);
        }
        let f = ed.doc.project.sprites[0].frames[0].rect;
        assert_eq!((f.w, f.h), (16, 8));
        assert_eq!(ed.doc.get(art(&ed), IVec2::new(1, 1)), 2, "the pixels stay put");
        ed.undo();
        let f = ed.doc.project.sprites[0].frames[0].rect;
        assert_eq!((f.w, f.h), (8, 8));
    }

    #[test]
    fn points_are_placed_by_clicking_in_the_frame() {
        let mut ed = editor();
        sprite_tab(&mut ed);
        ed.placing_point = Some("muzzle".into());
        click(&mut ed, 6, 3);
        assert_eq!(ed.doc.project.sprites[0].frames[0].points["muzzle"], IVec2::new(6, 3));
        assert!(ed.placing_point.is_none());
    }

    #[test]
    fn saving_writes_the_project_folder_and_reopens_it() {
        let dir = std::env::temp_dir().join(format!("pixelle-editor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut ed = editor();
        ed.open_project(&dir).unwrap();
        sprite_tab(&mut ed);
        ed.tool = Tool::Pencil;
        ed.primary = 4;
        click(&mut ed, 2, 2);
        assert!(ed.any_unsaved());
        ed.save();
        assert!(!ed.any_unsaved());
        let back = pixelle_core::ProjectDir::load(&dir).unwrap();
        assert_eq!(back.sprites["hero"].frames[0].pixels["art"].get(2, 2), 4);
        // Renaming and deleting take effect on disk at the next save.
        ed.rename("hero", "chars/hero").unwrap();
        ed.save();
        assert!(dir.join("chars/hero.pxl").exists());
        assert!(!dir.join("hero.pxl").exists());
        ed.delete("chars/hero");
        ed.save();
        assert!(!dir.join("chars/hero.pxl").exists());
        assert!(pixelle_core::ProjectDir::load(&dir).unwrap().sprites.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tabs_keep_their_own_canvas_and_history() {
        let mut ed = editor();
        sprite_tab(&mut ed);
        ed.tool = Tool::Pencil;
        ed.primary = 5;
        click(&mut ed, 0, 0);
        ed.activate(0);
        assert_eq!(ed.kind(), &TabKind::Sandbox);
        assert!(ed.doc.can_undo().is_none());
        ed.activate(1);
        assert_eq!(ed.doc.can_undo(), Some("Pencil"));
        assert_eq!(ed.doc.get(art(&ed), IVec2::ZERO), 5);
    }

    #[test]
    fn terrain_paints_cells_and_right_erases() {
        let mut ed = editor();
        let mut id = 0;
        ed.edit("layer", None, |p| {
            id = p.add_layer("Level", LayerKind::Terrain(pixelle_core::TerrainLayer { cell_size: 8, cells: Default::default() }))
        });
        ed.active_layer = Some(id);
        ed.tool = Tool::Terrain;
        ed.active_terrain = 1;
        drag(&mut ed, &[(0, 0), (40, 0)], PointerButton::Primary, Modifiers::NONE);
        for c in 0..=5 {
            assert_eq!(ed.doc.get(id, IVec2::new(c, 0)), 1, "cell {c}");
        }
        drag(&mut ed, &[(17, 3)], PointerButton::Secondary, Modifiers::NONE);
        assert_eq!(ed.doc.get(id, IVec2::new(2, 0)), 0);
    }

    #[test]
    fn colours_inserted_mid_palette_keep_the_art_of_every_file() {
        let mut ed = editor();
        sprite_tab(&mut ed);
        ed.tool = Tool::Pencil;
        ed.primary = 20;
        click(&mut ed, 0, 0);
        let before = ed.doc.project.palette.rgba(20);
        ed.add_palette_section("B", &[("b".into(), [1, 2, 3, 255])]);
        ed.activate(0);
        ed.insert_color(0, "new", [9, 9, 9, 255]).unwrap();
        ed.activate(1);
        let v = ed.doc.get(art(&ed), IVec2::ZERO);
        assert_eq!(ed.doc.project.palette.rgba(v), before);
        assert_eq!(ed.project.palette, ed.doc.project.palette);
        // The insert is one step of its own.
        ed.undo();
        assert_eq!(ed.doc.get(art(&ed), IVec2::ZERO), 20);
    }
}
