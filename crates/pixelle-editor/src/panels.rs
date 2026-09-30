//! The panels around the canvas.

use eframe::egui::{self, Color32, RichText, Sense, Stroke, StrokeKind, vec2};
use pixelle_core::layer::{Instance, LayerKind, TerrainLayer};
use pixelle_core::palette::presets;
use pixelle_core::tiles::{Cond, Rule, Terrain, Tileset, presets as tp};
use pixelle_core::{Bookmark, D4, Frame, Grid, IRect, IVec2, Project, SwapSlot};

use crate::app::{Editor, Selected};
use crate::canvas::{sprite_mut, unique_name};
use crate::tools::Tool;

fn color32(c: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

/// A palette swatch; transparent shows as a checker.
fn swatch(ui: &mut egui::Ui, c: [u8; 4], size: f32, primary: bool, secondary: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let p = ui.painter();
    if c[3] < 255 {
        let h = size / 2.0;
        p.rect_filled(rect, 0.0, Color32::from_gray(90));
        p.rect_filled(egui::Rect::from_min_size(rect.min, vec2(h, h)), 0.0, Color32::from_gray(150));
        p.rect_filled(egui::Rect::from_min_size(rect.min + vec2(h, h), vec2(h, h)), 0.0, Color32::from_gray(150));
    }
    p.rect_filled(rect, 0.0, color32(c));
    if primary {
        p.rect_stroke(rect, 0.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
    } else if secondary {
        p.rect_stroke(rect, 0.0, Stroke::new(2.0, Color32::from_gray(20)), StrokeKind::Inside);
        p.rect_stroke(rect.shrink(2.0), 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
    }
    if resp.hovered() {
        p.rect_stroke(rect, 0.0, Stroke::new(1.0, Color32::YELLOW), StrokeKind::Inside);
    }
    resp
}

const TRANSFORMS: [(&str, D4); 8] = [
    ("none", D4::new(false, false, false)),
    ("flip x", D4::new(false, true, false)),
    ("flip y", D4::new(false, false, true)),
    ("rotate 180", D4::new(false, true, true)),
    ("transpose", D4::new(true, false, false)),
    ("rotate 90 cw", D4::new(true, true, false)),
    ("rotate 90 ccw", D4::new(true, false, true)),
    ("anti-transpose", D4::new(true, true, true)),
];

fn transform_combo(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, t: &mut D4) -> bool {
    let mut changed = false;
    let name = TRANSFORMS.iter().find(|(_, d)| d == t).map_or("?", |(n, _)| n);
    egui::ComboBox::from_id_salt(id).selected_text(name).width(110.0).show_ui(ui, |ui| {
        for (n, d) in TRANSFORMS {
            changed |= ui.selectable_value(t, d, n).changed();
        }
    });
    changed
}

impl Editor {
    pub fn tools_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        egui::Grid::new("tools").num_columns(3).spacing([4.0, 4.0]).show(ui, |ui| {
            for (i, tool) in Tool::ALL.into_iter().enumerate() {
                let label = RichText::new(format!("{} {}", tool.key(), tool.name())).size(12.0);
                if ui.add_sized([74.0, 24.0], egui::Button::selectable(self.tool == tool, label)).on_hover_text(tool.hint()).clicked() {
                    if self.tool != tool {
                        self.commit_floating();
                    }
                    self.tool = tool;
                }
                if i % 3 == 2 {
                    ui.end_row();
                }
            }
        });
        ui.separator();

        // The two colours.
        let pal = &self.doc.project.palette;
        let (p, s) = (pal.rgba(self.primary), pal.rgba(self.secondary));
        ui.horizontal(|ui| {
            swatch(ui, p, 36.0, false, false).on_hover_text("Primary (left button)");
            swatch(ui, s, 24.0, false, false).on_hover_text("Secondary (right button)");
            if ui.small_button("<>").on_hover_text("Swap (X)").clicked() {
                std::mem::swap(&mut self.primary, &mut self.secondary);
            }
        });
        let name = |i: u8| pal.entry(i).map_or(String::new(), |e| format!("{i} {}", e.name));
        ui.label(RichText::new(name(self.primary)).small());
        ui.label(RichText::new(name(self.secondary)).small().weak());
        ui.separator();

        // What the current tool can be told.
        match self.tool {
            Tool::Pencil | Tool::Eraser => {
                ui.checkbox(&mut self.opts.pixel_perfect, "Pixel-perfect");
                ui.checkbox(&mut self.opts.mirror, "Mirror in frame").on_hover_text("Mirror left-right inside the sprite frame you start in");
            }
            Tool::Select => {
                ui.label("Selection");
                ui.horizontal_wrapped(|ui| {
                    for (label, t) in [("Flip X", D4::FLIP_X), ("Flip Y", D4::FLIP_Y), ("Rotate", D4::new(true, true, false))] {
                        if ui.button(label).clicked() {
                            self.transform_floating(t);
                        }
                    }
                });
                if self.floating.is_some() && ui.button("Put down (Enter)").clicked() {
                    self.commit_floating();
                }
                if let Some(sel) = self.selection {
                    ui.label(format!("{}×{} at {}, {}", sel.w, sel.h, sel.x, sel.y));
                    if ui.button("Make sprite from selection").clicked() {
                        self.sprite_from_selection(sel);
                    }
                    if ui.button("Make tileset from selection").clicked() {
                        self.tileset_from_selection(sel);
                    }
                    if ui.button("Bookmark selection").clicked() {
                        let name = format!("Area {}", self.doc.project.bookmarks.len() + 1);
                        self.edit("Add bookmark", None, |p| p.bookmarks.push(Bookmark { name, rect: sel }));
                    }
                }
            }
            Tool::Terrain => {
                ui.label("Paint with");
                let terrains: Vec<String> = self.doc.project.terrains.iter().map(|t| t.name.clone()).collect();
                for (i, name) in terrains.iter().enumerate() {
                    ui.radio_value(&mut self.active_terrain, i as u8 + 1, name);
                }
                if terrains.is_empty() {
                    ui.label(RichText::new("No terrains yet: make a tileset from a selection, then a terrain in the side panel").small());
                }
            }
            Tool::Sprite => {
                if let Some(name) = &self.placing_point {
                    ui.colored_label(Color32::YELLOW, format!("Click to place {name:?}"));
                }
            }
            Tool::Instance => {
                ui.label(RichText::new(match &self.last_sprite {
                    Some(s) => format!("Places {s}"),
                    None => "Select a sprite to place".into(),
                }).small());
            }
            _ => {}
        }
    }

    fn sprite_from_selection(&mut self, sel: IRect) {
        self.commit_floating();
        let Some(layer) = self.pixel_layer() else {
            self.say("Pick the pixel layer the sprite exports");
            return;
        };
        let name = unique_name(&self.doc.project, "sprite");
        let n = name.clone();
        self.edit("New sprite", None, |p| p.sprites.push(pixelle_core::Sprite::new(n, vec![layer], vec![Frame::new(sel)])));
        self.select_sprite(&name, 0);
    }

    fn tileset_from_selection(&mut self, sel: IRect) {
        self.commit_floating();
        let Some(layer) = self.pixel_layer() else {
            self.say("Pick the pixel layer the tiles are drawn on");
            return;
        };
        let g = self.doc.project.grid.max(1);
        let (cols, rows) = (sel.w / g, sel.h / g);
        if cols == 0 || rows == 0 {
            self.say(format!("A tileset is at least one {g}px tile"));
            return;
        }
        let name = (1..).map(|i| format!("tileset_{i}")).find(|n| self.doc.project.tileset(n).is_none()).unwrap();
        let ts = Tileset { name: name.clone(), origin: sel.min(), tile_size: g, columns: cols, count: cols * rows, layers: vec![layer] };
        self.edit("New tileset", None, |p| p.tilesets.push(ts));
        self.say(format!("Made {name} with {} tiles", cols * rows));
    }

    pub fn side_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            egui::CollapsingHeader::new("Layers").default_open(true).show(ui, |ui| self.layers_panel(ui));
            egui::CollapsingHeader::new("Inspector").default_open(true).show(ui, |ui| self.inspector(ui));
            egui::CollapsingHeader::new("Sprites").default_open(true).show(ui, |ui| self.sprites_panel(ui));
            egui::CollapsingHeader::new("Terrains").default_open(false).show(ui, |ui| self.terrains_panel(ui));
            egui::CollapsingHeader::new("Recolour slots").default_open(false).show(ui, |ui| self.swaps_panel(ui));
            egui::CollapsingHeader::new("Bookmarks").default_open(false).show(ui, |ui| self.bookmarks_panel(ui));
        });
    }

    // --- Palette -------------------------------------------------------------------------------

    pub fn palette_panel(&mut self, ui: &mut egui::Ui) {
        let sections = self.doc.project.palette.sections().to_vec();
        let mut clicked = None;
        // The transparent entry, then each section.
        ui.horizontal(|ui| {
            let r = swatch(ui, [0; 4], 16.0, self.primary == 0, self.secondary == 0).on_hover_text("0 transparent");
            if r.clicked() {
                clicked = Some((0, false));
            }
            if r.secondary_clicked() {
                clicked = Some((0, true));
            }
            ui.label(RichText::new("transparent").small().weak());
        });
        for (si, s) in sections.iter().enumerate() {
            let id = ui.make_persistent_id(("section", si));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                .show_header(ui, |ui| {
                    ui.label(RichText::new(&s.label).strong());
                    ui.label(RichText::new(format!("{}", s.len)).small().weak());
                })
                .body(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                        for i in s.indices() {
                            let e = self.doc.project.palette.entry(i).unwrap();
                            let r = swatch(ui, e.rgba, 18.0, self.primary == i, self.secondary == i);
                            let r = r.on_hover_text(format!("{i} {}  #{:02x}{:02x}{:02x}{:02x}", e.name, e.rgba[0], e.rgba[1], e.rgba[2], e.rgba[3]));
                            if r.clicked() {
                                clicked = Some((i, false));
                            }
                            if r.secondary_clicked() {
                                clicked = Some((i, true));
                            }
                        }
                        if ui.small_button("+").on_hover_text("Add a colour to this section").clicked() {
                            let c = self.doc.project.palette.rgba(self.primary);
                            let c = if c[3] == 0 { [255, 255, 255, 255] } else { c };
                            let mut new = 0;
                            self.edit("Add colour", None, |p| new = p.insert_color(si, "new", c).unwrap_or(0));
                            if new != 0 {
                                self.primary = new;
                            }
                        }
                    });
                });
        }
        if let Some((i, secondary)) = clicked {
            if secondary {
                self.secondary = i;
            } else {
                self.primary = i;
            }
        }

        // The primary entry, editable.
        let i = self.primary;
        ui.separator();
        if i == 0 {
            ui.label(RichText::new("Pick a colour to edit it").weak());
        } else if let Some(entry) = self.doc.project.palette.entry(i).cloned() {
            let section = self.doc.project.palette.section_of(i).map_or("—".to_string(), |s| s.label.clone());
            ui.horizontal(|ui| {
                ui.label(format!("#{i} in {section}"));
                ui.label(RichText::new(format!("{} px", self.doc.project.color_usage(i))).weak());
            });
            let mut name = entry.name.clone();
            if ui.text_edit_singleline(&mut name).changed() {
                self.edit("Rename colour", Some(&format!("name-{i}")), |p| p.palette.entry_mut(i).unwrap().name = name);
            }
            let mut rgba = entry.rgba;
            ui.horizontal(|ui| {
                if ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed() {
                    self.edit("Edit colour", Some(&format!("colour-{i}")), |p| p.palette.entry_mut(i).unwrap().rgba = rgba);
                }
                let mut hex = format!("{:02x}{:02x}{:02x}{:02x}", rgba[0], rgba[1], rgba[2], rgba[3]);
                if ui.add(egui::TextEdit::singleline(&mut hex).desired_width(70.0)).changed()
                    && let Ok(v) = u32::from_str_radix(hex.trim_start_matches('#'), 16)
                {
                    let c = if hex.len() > 6 { v.to_be_bytes() } else { (v << 8 | 0xff).to_be_bytes() };
                    self.edit("Edit colour", Some(&format!("colour-{i}")), |p| p.palette.entry_mut(i).unwrap().rgba = c);
                }
            });
            ui.horizontal(|ui| {
                let mut on = self.highlight == Some(i);
                if ui.toggle_value(&mut on, "Highlight").on_hover_text("Flash every pixel of this colour").changed() {
                    self.highlight = on.then_some(i);
                }
                if ui.button("Delete").on_hover_text("Pixels of it become the secondary colour").clicked() {
                    let to = self.secondary;
                    self.edit("Delete colour", None, |p| p.remove_color(i, to));
                    self.primary = 0;
                    self.fix_selection();
                }
                if ui.button("Make recolour slot").clicked() {
                    let name = entry.name.clone();
                    let options = self.doc.project.palette.sections().first().map_or(String::new(), |s| s.label.clone());
                    self.edit("Add recolour slot", None, |p| p.swaps.push(SwapSlot { name, index: i, options }));
                }
            });
        }
        ui.horizontal(|ui| {
            ui.menu_button("Add preset…", |ui| {
                for preset in presets::ALL {
                    if ui.button(preset.0).clicked() {
                        let colours: Vec<(String, [u8; 4])> = preset
                            .1
                            .iter()
                            .map(|v| (format!("#{v:06x}"), [(v >> 16) as u8, (v >> 8) as u8, *v as u8, 255]))
                            .collect();
                        self.add_palette_section(preset.0, &colours);
                    }
                }
            });
            if ui.button("Import…").clicked() {
                self.import_palette();
            }
            if ui.button("New section").clicked() {
                let label = format!("Section {}", self.doc.project.palette.sections().len() + 1);
                self.add_palette_section(&label, &[]);
            }
        });
        if let Some(si) = self.doc.project.palette.sections().iter().position(|s| s.contains(self.primary)) {
            let mut label = self.doc.project.palette.sections()[si].label.clone();
            ui.horizontal(|ui| {
                ui.label("Section name");
                if ui.text_edit_singleline(&mut label).changed() {
                    self.edit("Rename section", Some(&format!("section-{si}")), |p| p.palette.rename_section(si, label));
                }
            });
        }
    }

    // --- Layers --------------------------------------------------------------------------------

    fn layers_panel(&mut self, ui: &mut egui::Ui) {
        let layers: Vec<(u32, String, bool, bool, bool, &'static str)> = self
            .doc
            .project
            .layers
            .iter()
            .rev()
            .map(|l| (l.id, l.name.clone(), l.visible, l.locked, l.reference, l.kind_name()))
            .collect();
        for (id, name, visible, locked, reference, kind) in layers {
            ui.horizontal(|ui| {
                let mut v = visible;
                if ui.toggle_value(&mut v, if visible { "👁" } else { "–" }).on_hover_text("Visible").changed() {
                    self.edit("Toggle visibility", None, |p| p.layer_mut(id).unwrap().visible = v);
                }
                let mut k = locked;
                if ui.toggle_value(&mut k, if locked { "🔒" } else { "🔓" }).on_hover_text("Locked").changed() {
                    self.edit("Toggle lock", None, |p| p.layer_mut(id).unwrap().locked = k);
                }
                let active = self.active_layer == Some(id);
                let text = RichText::new(&name);
                let text = if reference { text.italics() } else { text };
                if ui.selectable_label(active, text).clicked() {
                    self.commit_floating();
                    self.active_layer = Some(id);
                    // Put the matching tool in hand.
                    match kind {
                        "terrain" if !matches!(self.tool, Tool::Terrain | Tool::Hand) => self.tool = Tool::Terrain,
                        "instances" if !matches!(self.tool, Tool::Instance | Tool::Hand) => self.tool = Tool::Instance,
                        "pixels" if matches!(self.tool, Tool::Terrain | Tool::Instance) => self.tool = Tool::Pencil,
                        _ => {}
                    }
                }
                ui.label(RichText::new(if reference { format!("{kind} · ref") } else { kind.to_string() }).small().weak());
            });
        }
        ui.horizontal_wrapped(|ui| {
            let mut add = |ui: &mut egui::Ui, label: &str, kind: fn(&Project) -> LayerKind| {
                if ui.button(label).clicked() {
                    let mut id = 0;
                    self.edit("Add layer", None, |p| {
                        let k = kind(p);
                        let n = p.layers.len() + 1;
                        id = p.add_layer(format!("Layer {n}"), k);
                    });
                    self.active_layer = Some(id);
                }
            };
            add(ui, "+ Pixels", |_| LayerKind::Pixels(Grid::new()));
            add(ui, "+ Terrain", |p| LayerKind::Terrain(TerrainLayer { cell_size: p.grid, cells: Grid::new() }));
            add(ui, "+ Instances", |_| LayerKind::Instances(Vec::new()));
        });
        let Some(id) = self.active_layer else { return };
        let Some(layer) = self.doc.project.layer(id).cloned() else { return };
        ui.horizontal(|ui| {
            let mut name = layer.name.clone();
            if ui.add(egui::TextEdit::singleline(&mut name).desired_width(120.0)).changed() {
                self.edit("Rename layer", Some(&format!("layer-name-{id}")), |p| p.layer_mut(id).unwrap().name = name);
            }
            let mut r = layer.reference;
            if ui.checkbox(&mut r, "Reference").on_hover_text("Shown, never exported").changed() {
                self.edit("Toggle reference", None, |p| p.layer_mut(id).unwrap().reference = r);
            }
        });
        ui.horizontal(|ui| {
            let pos = self.doc.project.layers.iter().position(|l| l.id == id).unwrap_or(0);
            if ui.add_enabled(pos + 1 < self.doc.project.layers.len(), egui::Button::new("Up")).clicked() {
                self.edit("Move layer", None, |p| p.layers.swap(pos, pos + 1));
            }
            if ui.add_enabled(pos > 0, egui::Button::new("Down")).clicked() {
                self.edit("Move layer", None, |p| p.layers.swap(pos, pos - 1));
            }
            if ui.button("Delete").clicked() {
                self.edit("Delete layer", None, |p| {
                    p.layers.retain(|l| l.id != id);
                    for s in &mut p.sprites {
                        s.layers.retain(|l| *l != id);
                    }
                    for t in &mut p.tilesets {
                        t.layers.retain(|l| *l != id);
                    }
                });
                self.fix_selection();
            }
            if let LayerKind::Terrain(t) = &layer.kind {
                let mut size = t.cell_size;
                ui.label("cell");
                if ui.add(egui::DragValue::new(&mut size).range(1..=256)).changed() {
                    self.edit("Cell size", Some("cell-size"), |p| {
                        if let LayerKind::Terrain(t) = &mut p.layer_mut(id).unwrap().kind {
                            t.cell_size = size;
                        }
                    });
                }
            }
        });
    }

    // --- Sprites -------------------------------------------------------------------------------

    fn sprites_panel(&mut self, ui: &mut egui::Ui) {
        ui.add(egui::TextEdit::singleline(&mut self.sprite_filter).hint_text("filter"));
        let filter = self.sprite_filter.to_lowercase();
        let names: Vec<(String, bool, usize)> = self
            .doc
            .project
            .sprites
            .iter()
            .filter(|s| filter.is_empty() || s.name.to_lowercase().contains(&filter))
            .map(|s| (s.name.clone(), s.is_animation(), s.frames.len()))
            .collect();
        let selected = match &self.selected {
            Selected::Sprite { name, .. } => Some(name.clone()),
            _ => None,
        };
        egui::ScrollArea::vertical().id_salt("sprites").max_height(260.0).show(ui, |ui| {
            for (name, anim, frames) in names {
                let label = if anim {
                    format!("▶ {name}  ({frames})")
                } else if frames > 1 {
                    format!("# {name}  ({frames})")
                } else {
                    format!("  {name}")
                };
                let r = ui.selectable_label(selected.as_deref() == Some(&name), label);
                if r.clicked() {
                    self.select_sprite(&name, 0);
                }
                if r.double_clicked()
                    && let Some(s) = self.doc.project.sprite(&name)
                {
                    let b = s.bounds();
                    self.view.fit(self.canvas_rect, b.grow(b.w.max(b.h) as i32));
                }
            }
        });
        ui.label(RichText::new("Double-click to go to it · drag on the canvas with the Sprite tool to make one").small().weak());
    }

    // --- Inspector -----------------------------------------------------------------------------

    fn inspector(&mut self, ui: &mut egui::Ui) {
        match self.selected.clone() {
            Selected::None => {
                ui.label(RichText::new("Select a sprite, an instance or a terrain").weak());
            }
            Selected::Sprite { name, frame } => self.sprite_inspector(ui, &name, frame),
            Selected::Instance { layer, index } => self.instance_inspector(ui, layer, index),
            Selected::Terrain(i) => self.terrain_inspector(ui, i),
        }
    }

    fn sprite_inspector(&mut self, ui: &mut egui::Ui, name: &str, frame: usize) {
        let Some(sprite) = self.doc.project.sprite(name).cloned() else { return };
        let key = |what: &str| format!("{what}-{name}");

        ui.horizontal(|ui| {
            ui.label("Name");
            let mut new = sprite.name.clone();
            let r = ui.text_edit_singleline(&mut new);
            if r.lost_focus() && new != sprite.name && !new.is_empty() {
                if self.doc.project.sprite(&new).is_some() {
                    self.say(format!("{new} already exists"));
                } else {
                    let old = sprite.name.clone();
                    let n = new.clone();
                    self.edit("Rename sprite", None, |p| {
                        sprite_mut(p, &old).unwrap().name = n.clone();
                        for l in &mut p.layers {
                            if let LayerKind::Instances(list) = &mut l.kind {
                                for i in list.iter_mut().filter(|i| i.sprite == old) {
                                    i.sprite = n.clone();
                                }
                            }
                        }
                    });
                    self.select_sprite(&new, frame);
                }
            }
        });

        // Which layers it exports.
        ui.horizontal_wrapped(|ui| {
            ui.label("Layers");
            let layers: Vec<(u32, String)> = self
                .doc
                .project
                .layers
                .iter()
                .filter(|l| !matches!(l.kind, LayerKind::Instances(_)))
                .map(|l| (l.id, l.name.clone()))
                .collect();
            for (id, lname) in layers {
                let mut on = sprite.layers.contains(&id);
                if ui.checkbox(&mut on, lname).changed() {
                    let n = name.to_string();
                    self.edit("Sprite layers", None, |p| {
                        let s = sprite_mut(p, &n).unwrap();
                        if on {
                            s.layers.push(id);
                        } else {
                            s.layers.retain(|l| *l != id);
                        }
                    });
                }
            }
        });

        // Frames.
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(format!("{} frame(s)", sprite.frames.len()));
            if ui.button("+ Frame →").on_hover_text("Add a frame right of the last one").clicked() {
                let n = name.to_string();
                self.edit("Add frame", None, |p| {
                    let s = sprite_mut(p, &n).unwrap();
                    let last = s.frames.last().cloned().unwrap();
                    let mut f = Frame::new(last.rect.offset(IVec2::new(last.rect.w as i32, 0)));
                    f.duration = if last.duration > 0.0 { last.duration } else { 0.1 };
                    if last.duration == 0.0 {
                        for fr in &mut s.frames {
                            fr.duration = 0.1;
                        }
                    }
                    f.points = last.points.clone();
                    s.frames.push(f);
                });
            }
            if sprite.frames.len() > 1 && ui.button("- Frame").clicked() {
                let n = name.to_string();
                self.edit("Remove frame", None, |p| {
                    sprite_mut(p, &n).unwrap().frames.remove(frame);
                });
                self.select_sprite(name, frame.saturating_sub(1));
            }
            if ui.button("Delete sprite").clicked() {
                let n = name.to_string();
                self.edit("Delete sprite", None, |p| p.sprites.retain(|s| s.name != n));
                self.selected = Selected::None;
            }
        });
        ui.horizontal_wrapped(|ui| {
            for i in 0..sprite.frames.len() {
                if ui.selectable_label(i == frame, format!("{i}")).clicked() {
                    self.select_sprite(name, i);
                }
            }
        });
        let Some(f) = sprite.frames.get(frame).cloned() else { return };
        egui::Grid::new("frame").num_columns(2).show(ui, |ui| {
            let mut r = f.rect;
            ui.label("Rect");
            let changed = ui
                .horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut r.x).prefix("x ")).changed()
                        | ui.add(egui::DragValue::new(&mut r.y).prefix("y ")).changed()
                        | ui.add(egui::DragValue::new(&mut r.w).prefix("w ").range(1..=4096)).changed()
                        | ui.add(egui::DragValue::new(&mut r.h).prefix("h ").range(1..=4096)).changed()
                })
                .inner;
            if changed {
                let n = name.to_string();
                self.edit("Frame rect", Some(&key("rect")), |p| sprite_mut(p, &n).unwrap().frames[frame].rect = r);
            }
            ui.end_row();
            ui.label("Duration");
            let mut ms = (f.duration * 1000.0).round() as u32;
            if ui.add(egui::DragValue::new(&mut ms).suffix(" ms").range(0..=10_000)).on_hover_text("0 for stills").changed() {
                let n = name.to_string();
                self.edit("Frame duration", Some(&key("dur")), |p| sprite_mut(p, &n).unwrap().frames[frame].duration = ms as f32 / 1000.0);
            }
            ui.end_row();
            ui.label("Orientation");
            let mut t = f.transform;
            if transform_combo(ui, ("frame-t", name, frame), &mut t) {
                let n = name.to_string();
                self.edit("Frame orientation", None, |p| sprite_mut(p, &n).unwrap().frames[frame].transform = t);
            }
            ui.end_row();
        });

        // Anchor points.
        ui.label(RichText::new("Points").strong());
        for (pname, p) in &f.points {
            ui.horizontal(|ui| {
                ui.label(pname);
                let mut v = *p;
                let changed = ui.add(egui::DragValue::new(&mut v.x).prefix("x ")).changed() | ui.add(egui::DragValue::new(&mut v.y).prefix("y ")).changed();
                if changed {
                    let (n, pn) = (name.to_string(), pname.clone());
                    self.edit("Move point", Some(&key(pname)), |pr| {
                        sprite_mut(pr, &n).unwrap().frames[frame].points.insert(pn, v);
                    });
                }
                if ui.small_button("place").clicked() {
                    self.placing_point = Some(pname.clone());
                    self.tool = Tool::Sprite;
                }
                if ui.small_button("all frames").on_hover_text("Copy to every frame").clicked() {
                    let (n, pn) = (name.to_string(), pname.clone());
                    self.edit("Copy point", None, |pr| {
                        for fr in &mut sprite_mut(pr, &n).unwrap().frames {
                            fr.points.insert(pn.clone(), v);
                        }
                    });
                }
                if ui.small_button("×").clicked() {
                    let (n, pn) = (name.to_string(), pname.clone());
                    self.edit("Remove point", None, |pr| {
                        sprite_mut(pr, &n).unwrap().frames[frame].points.remove(&pn);
                    });
                }
            });
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_point_name).hint_text("point name").desired_width(100.0));
            if ui.button("Add & place").clicked() && !self.new_point_name.is_empty() {
                self.placing_point = Some(std::mem::take(&mut self.new_point_name));
                self.tool = Tool::Sprite;
            }
        });
        if !f.rects.is_empty() {
            ui.label(RichText::new("Rects").strong());
            for (rname, r) in &f.rects {
                ui.label(format!("{rname}: {}, {} {}×{}", r.x, r.y, r.w, r.h));
            }
        }

        // Pivot and tags.
        ui.horizontal(|ui| {
            let mut has = sprite.pivot.is_some();
            if ui.checkbox(&mut has, "Pivot").changed() {
                let n = name.to_string();
                let c = IVec2::new(f.rect.w as i32 / 2, f.rect.h as i32 / 2);
                self.edit("Pivot", None, |p| sprite_mut(p, &n).unwrap().pivot = has.then_some(c));
            }
            if let Some(mut pv) = sprite.pivot {
                let changed = ui.add(egui::DragValue::new(&mut pv.x).prefix("x ")).changed() | ui.add(egui::DragValue::new(&mut pv.y).prefix("y ")).changed();
                if changed {
                    let n = name.to_string();
                    self.edit("Pivot", Some(&key("pivot")), |p| sprite_mut(p, &n).unwrap().pivot = Some(pv));
                }
            }
        });
        for (k, v) in &sprite.tags {
            ui.horizontal(|ui| {
                ui.label(RichText::new(k).monospace());
                let mut val = v.clone();
                if ui.text_edit_singleline(&mut val).changed() {
                    let (n, k2) = (name.to_string(), k.clone());
                    self.edit("Edit tag", Some(&key(k)), |p| {
                        sprite_mut(p, &n).unwrap().tags.insert(k2, val);
                    });
                }
            });
        }
    }

    fn instance_inspector(&mut self, ui: &mut egui::Ui, layer: u32, index: usize) {
        let Some(inst) = self.instance(layer, index).cloned() else { return };
        let edit = |ed: &mut Editor, label: &str, key: Option<&str>, f: &dyn Fn(&mut Instance)| {
            ed.edit(label, key, |p| {
                if let Some(LayerKind::Instances(list)) = p.layer_mut(layer).map(|l| &mut l.kind)
                    && let Some(i) = list.get_mut(index)
                {
                    f(i);
                }
            });
        };
        ui.horizontal(|ui| {
            ui.label("Sprite");
            let names: Vec<String> = self.doc.project.sprites.iter().map(|s| s.name.clone()).collect();
            let mut s = inst.sprite.clone();
            egui::ComboBox::from_id_salt("inst-sprite").selected_text(&s).width(170.0).show_ui(ui, |ui| {
                for n in names {
                    ui.selectable_value(&mut s, n.clone(), n);
                }
            });
            if s != inst.sprite {
                edit(self, "Instance sprite", None, &|i| {
                    i.sprite = s.clone();
                    i.frame = 0;
                });
            }
        });
        let frames = self.doc.project.sprite(&inst.sprite).map_or(1, |s| s.frames.len());
        ui.horizontal(|ui| {
            let mut fr = inst.frame;
            ui.label("Frame");
            if ui.add(egui::DragValue::new(&mut fr).range(0..=frames.saturating_sub(1) as u32)).changed() {
                edit(self, "Instance frame", Some("inst-frame"), &|i| i.frame = fr);
            }
            let mut pos = inst.pos;
            ui.label("At");
            let changed = ui.add(egui::DragValue::new(&mut pos.x).prefix("x ")).changed() | ui.add(egui::DragValue::new(&mut pos.y).prefix("y ")).changed();
            if changed {
                edit(self, "Move instance", Some("inst-pos"), &|i| i.pos = pos);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Orientation");
            let mut t = inst.transform;
            if transform_combo(ui, "inst-t", &mut t) {
                edit(self, "Instance orientation", None, &|i| i.transform = t);
            }
        });
        // Recolouring.
        let slots = self.doc.project.swaps.clone();
        for slot in slots {
            let current = inst.swaps.iter().find(|(n, _)| *n == slot.name).map(|(_, v)| *v);
            ui.horizontal(|ui| {
                ui.label(&slot.name);
                let Some(sec) = self.doc.project.palette.section(&slot.options).cloned() else { return };
                ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                let default = swatch(ui, self.doc.project.palette.rgba(slot.index), 14.0, current.is_none(), false).on_hover_text("as drawn");
                if default.clicked() {
                    let n = slot.name.clone();
                    edit(self, "Recolour", None, &|i| i.swaps.retain(|(s, _)| *s != n));
                }
                for c in sec.indices() {
                    let r = swatch(ui, self.doc.project.palette.rgba(c), 14.0, current == Some(c), false);
                    if r.clicked() {
                        let n = slot.name.clone();
                        edit(self, "Recolour", None, &|i| {
                            i.swaps.retain(|(s, _)| *s != n);
                            i.swaps.push((n.clone(), c));
                        });
                    }
                }
            });
        }
        if ui.button("Delete instance").clicked() {
            self.edit("Delete instance", None, |p| {
                if let Some(LayerKind::Instances(list)) = p.layer_mut(layer).map(|l| &mut l.kind) {
                    list.remove(index);
                }
            });
            self.selected = Selected::None;
        }
    }

    // --- Terrains ------------------------------------------------------------------------------

    fn terrains_panel(&mut self, ui: &mut egui::Ui) {
        let terrains: Vec<(String, String)> = self.doc.project.terrains.iter().map(|t| (t.name.clone(), t.tileset.clone())).collect();
        for (i, (name, ts)) in terrains.iter().enumerate() {
            let sel = self.selected == Selected::Terrain(i);
            if ui.selectable_label(sel, format!("{} {name}  on {ts}", i + 1)).clicked() {
                self.selected = Selected::Terrain(i);
                self.active_terrain = i as u8 + 1;
            }
        }
        let tilesets: Vec<(String, u32)> = self.doc.project.tilesets.iter().map(|t| (t.name.clone(), t.count)).collect();
        if tilesets.is_empty() {
            ui.label(RichText::new("Select tiles on the canvas (Select tool) and choose “Make tileset from selection”").small().weak());
            return;
        }
        ui.menu_button("New terrain…", |ui| {
            for (ts, count) in &tilesets {
                ui.menu_button(format!("on {ts} ({count} tiles)"), |ui| {
                    // Name, tiles needed, rules.
                    type Preset = (&'static str, u32, fn() -> Vec<Rule>);
                    let options: [Preset; 4] = [
                        ("Minimal D4 (7 tiles)", 7, || {
                            tp::minimal_d4(tp::MinimalD4 { corner: 0, edge: 1, strip_end: 2, strip: 3, interior: 4, inside: 5, double_inside: 6 })
                        }),
                        ("Edge 16", 16, tp::edge16),
                        ("Blob 47", 47, tp::blob47),
                        ("Horizontal run (3 tiles)", 3, || tp::horizontal_run(0, 1, 2, 1)),
                    ];
                    for (label, need, rules) in options {
                        let enabled = *count >= need;
                        if ui.add_enabled(enabled, egui::Button::new(label)).on_disabled_hover_text(format!("needs {need} tiles")).clicked() {
                            let name = format!("{ts} terrain");
                            let t = Terrain { name, tileset: ts.clone(), rules: rules(), fallback: None };
                            self.edit("New terrain", None, |p| p.terrains.push(t));
                            let i = self.doc.project.terrains.len() - 1;
                            self.selected = Selected::Terrain(i);
                            self.active_terrain = i as u8 + 1;
                        }
                    }
                });
            }
        });
        ui.label(RichText::new("Presets expect the tileset's tiles in their order: see the rules in the inspector").small().weak());
    }

    fn terrain_inspector(&mut self, ui: &mut egui::Ui, ti: usize) {
        let Some(t) = self.doc.project.terrains.get(ti).cloned() else { return };
        ui.horizontal(|ui| {
            ui.label("Name");
            let mut name = t.name.clone();
            if ui.text_edit_singleline(&mut name).changed() {
                self.edit("Rename terrain", Some(&format!("terrain-name-{ti}")), |p| p.terrains[ti].name = name);
            }
        });
        let count = self.doc.project.tileset(&t.tileset).map_or(0, |ts| ts.count);
        ui.horizontal(|ui| {
            ui.label(format!("Tileset {} · {count} tiles", t.tileset));
            let mut fb = t.fallback.is_some();
            if ui.checkbox(&mut fb, "fallback").changed() {
                self.edit("Terrain fallback", None, |p| p.terrains[ti].fallback = fb.then_some(0));
            }
            if let Some(mut f) = t.fallback
                && ui.add(egui::DragValue::new(&mut f).range(0..=count.saturating_sub(1))).changed()
            {
                self.edit("Terrain fallback", Some("fallback"), |p| p.terrains[ti].fallback = Some(f));
            }
        });
        ui.label(RichText::new("Rules, first match wins. Click a neighbour: ? any · # same · . other").small().weak());
        let ts = self.doc.project.tileset(&t.tileset).cloned();
        for (ri, rule) in t.rules.iter().enumerate() {
            ui.horizontal(|ui| {
                // The 3x3 neighbourhood.
                egui::Grid::new(("rule", ti, ri)).spacing([1.0, 1.0]).show(ui, |ui| {
                    let mut slot = 0;
                    for y in 0..3 {
                        for x in 0..3 {
                            if (x, y) == (1, 1) {
                                ui.label(RichText::new("x").small());
                            } else {
                                let s = slot;
                                let c = rule.pattern[s];
                                let label = match c {
                                    Cond::Any => "?",
                                    Cond::Same => "#",
                                    Cond::NotSame => ".",
                                };
                                if ui.add_sized([16.0, 16.0], egui::Button::new(RichText::new(label).small())).clicked() {
                                    let next = match c {
                                        Cond::Any => Cond::Same,
                                        Cond::Same => Cond::NotSame,
                                        Cond::NotSame => Cond::Any,
                                    };
                                    self.edit("Edit rule", None, |p| p.terrains[ti].rules[ri].pattern[s] = next);
                                }
                                slot += 1;
                            }
                        }
                        ui.end_row();
                    }
                });
                ui.vertical(|ui| {
                    ui.label(RichText::new(&rule.name).small());
                    ui.horizontal(|ui| {
                        let mut tile = rule.tile;
                        if ui.add(egui::DragValue::new(&mut tile).prefix("tile ").range(0..=count.saturating_sub(1))).changed() {
                            self.edit("Edit rule", Some(&format!("rule-tile-{ti}-{ri}")), |p| p.terrains[ti].rules[ri].tile = tile);
                        }
                        if ts.is_some()
                            && let Some(tex) = self.backend.tile(ui.ctx(), &self.doc.project, &t.tileset, rule.tile, rule.transform)
                        {
                            ui.image((tex, vec2(20.0, 20.0)));
                        }
                    });
                    ui.horizontal(|ui| {
                        let mut sym = rule.symmetric;
                        if ui.checkbox(&mut sym, "all 8 turns").changed() {
                            self.edit("Edit rule", None, |p| p.terrains[ti].rules[ri].symmetric = sym);
                        }
                        let mut tr = rule.transform;
                        if transform_combo(ui, ("rule-t", ti, ri), &mut tr) {
                            self.edit("Edit rule", None, |p| p.terrains[ti].rules[ri].transform = tr);
                        }
                        if ui.small_button("×").clicked() {
                            self.edit("Remove rule", None, |p| {
                                p.terrains[ti].rules.remove(ri);
                            });
                        }
                    });
                });
            });
        }
        ui.horizontal(|ui| {
            if ui.button("+ Rule").clicked() {
                self.edit("Add rule", None, |p| {
                    let r = Rule::parse("rule", "??? ?x? ???", 0).expect("valid");
                    p.terrains[ti].rules.push(r);
                });
            }
            if ui.button("Delete terrain").clicked() {
                self.edit("Delete terrain", None, |p| {
                    p.terrains.remove(ti);
                    // Cells hold terrain numbers: shift those above it down, clear its own.
                    let gone = ti as u8 + 1;
                    for l in &mut p.layers {
                        if let LayerKind::Terrain(t) = &mut l.kind {
                            t.cells.map_values(|v| if v == gone { 0 } else if v > gone { v - 1 } else { v });
                        }
                    }
                });
                self.selected = Selected::None;
            }
        });
    }

    // --- Swap slots and bookmarks --------------------------------------------------------------

    fn swaps_panel(&mut self, ui: &mut egui::Ui) {
        let slots = self.doc.project.swaps.clone();
        let sections: Vec<String> = self.doc.project.palette.sections().iter().map(|s| s.label.clone()).collect();
        for (i, slot) in slots.iter().enumerate() {
            ui.horizontal(|ui| {
                swatch(ui, self.doc.project.palette.rgba(slot.index), 16.0, false, false).on_hover_text(format!("index {}", slot.index));
                let mut name = slot.name.clone();
                if ui.add(egui::TextEdit::singleline(&mut name).desired_width(70.0)).changed() {
                    self.edit("Rename slot", Some(&format!("slot-{i}")), |p| p.swaps[i].name = name);
                }
                let mut opt = slot.options.clone();
                egui::ComboBox::from_id_salt(("slot-opt", i)).selected_text(&opt).width(110.0).show_ui(ui, |ui| {
                    for s in &sections {
                        ui.selectable_value(&mut opt, s.clone(), s);
                    }
                });
                if opt != slot.options {
                    self.edit("Slot options", None, |p| p.swaps[i].options = opt);
                }
                if ui.small_button("×").clicked() {
                    self.edit("Remove slot", None, |p| {
                        p.swaps.remove(i);
                    });
                }
            });
        }
        ui.label(RichText::new("A slot is a palette index the game may recolour from a section. Make one from a colour in the palette panel.").small().weak());
    }

    fn bookmarks_panel(&mut self, ui: &mut egui::Ui) {
        let marks = self.doc.project.bookmarks.clone();
        for (i, b) in marks.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.button("Go").clicked() {
                    self.view.fit(self.canvas_rect, b.rect);
                }
                let mut name = b.name.clone();
                if ui.add(egui::TextEdit::singleline(&mut name).desired_width(120.0)).changed() {
                    self.edit("Rename bookmark", Some(&format!("bm-{i}")), |p| p.bookmarks[i].name = name);
                }
                if ui.small_button("×").clicked() {
                    self.edit("Remove bookmark", None, |p| {
                        p.bookmarks.remove(i);
                    });
                }
            });
        }
        if ui.button("Bookmark the view").clicked() {
            let rect = self.view.visible(self.canvas_rect);
            let name = format!("View {}", marks.len() + 1);
            self.edit("Add bookmark", None, |p| p.bookmarks.push(Bookmark { name, rect }));
        }
    }
}
