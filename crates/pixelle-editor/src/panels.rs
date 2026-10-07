//! The panels around the canvas.

use eframe::egui::{self, Color32, RichText, Sense, Stroke, StrokeKind, vec2};
use pixelle_core::folder::{self, SpriteFrame};
use pixelle_core::layer::{LayerKind, TerrainLayer};
use pixelle_core::palette::presets;
use pixelle_core::{D4, Grid, IVec2, Project, SwapSlot};

use crate::app::{Editor, Modal, Selected};
use crate::canvas::sprite_mut;
use crate::tools::Tool;
use crate::workspace::{self, TabKind};

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
                }
            }
            Tool::Terrain => {
                ui.label("Paint with");
                let terrains: Vec<String> = self.doc.project.terrains.iter().map(|t| t.name.clone()).collect();
                for (i, name) in terrains.iter().enumerate() {
                    ui.radio_value(&mut self.active_terrain, i as u8 + 1, name);
                }
                if terrains.is_empty() {
                    ui.label(RichText::new("No terrains yet: make one with File → New file…").small());
                }
            }
            Tool::Sprite => {
                if let Some(name) = &self.placing_point {
                    ui.colored_label(Color32::YELLOW, format!("Click to place {name:?}"));
                }
            }
            _ => {}
        }
    }

    pub fn side_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            egui::CollapsingHeader::new("Layers").default_open(true).show(ui, |ui| self.layers_panel(ui));
            egui::CollapsingHeader::new("Inspector").default_open(true).show(ui, |ui| self.inspector(ui));
            egui::CollapsingHeader::new("Recolour slots").default_open(false).show(ui, |ui| self.swaps_panel(ui));
        });
    }

    // --- Files ---------------------------------------------------------------------------------

    pub fn files_panel(&mut self, ui: &mut egui::Ui) {
        let Some(dir) = self.disk.as_ref().map(|d| d.dir.clone()) else {
            ui.label(RichText::new("No project folder open: work here is not saved.").weak());
            if ui.button("Open or start a project folder…").clicked() {
                self.pick_project();
            }
            return;
        };
        ui.horizontal(|ui| {
            let name = dir.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
            let root = format!("{name}{}", if self.is_unsaved("project") || !self.pending_delete.is_empty() { " •" } else { "" });
            ui.label(RichText::new(root).strong()).on_hover_text(dir.display().to_string());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("+ New").on_hover_text("New file (⌘N)").clicked() {
                    self.ask_new_file();
                }
            });
        });
        ui.add(egui::TextEdit::singleline(&mut self.file_filter).hint_text("filter").desired_width(f32::INFINITY));
        let filter = self.file_filter.to_lowercase();
        let assets: Vec<(String, folder::Kind)> = self
            .project
            .assets()
            .into_iter()
            .filter(|a| filter.is_empty() || a.to_lowercase().contains(&filter))
            .filter_map(|a| self.project.kind(&a).map(|k| (a, k)))
            .collect();
        if assets.is_empty() {
            ui.label(RichText::new(if filter.is_empty() { "No files yet: + New makes a sprite or a terrain" } else { "Nothing matches" }).small().weak());
        }
        // A folder heading wherever the path changes folder.
        let open = self.kind().asset().map(str::to_string);
        let mut last_dir: Vec<String> = Vec::new();
        let mut action: Option<FileAction> = None;
        egui::ScrollArea::vertical().id_salt("files").max_height(260.0).show(ui, |ui| {
            for (name, kind) in &assets {
                let parts: Vec<&str> = name.split('/').collect();
                let (leaf, dirs) = parts.split_last().expect("names are not empty");
                let common = last_dir.iter().zip(dirs.iter()).take_while(|(a, b)| a == b).count();
                for (depth, d) in dirs.iter().enumerate().skip(common) {
                    ui.horizontal(|ui| {
                        ui.add_space(depth as f32 * 12.0);
                        ui.label(RichText::new(format!("📁 {d}")).weak());
                    });
                }
                last_dir = dirs.iter().map(|d| d.to_string()).collect();
                ui.horizontal(|ui| {
                    ui.add_space(dirs.len() as f32 * 12.0);
                    let icon = match kind {
                        folder::Kind::Sprite => "🖼",
                        folder::Kind::Terrain => "▦",
                    };
                    let dirty = if self.is_unsaved(name) { " •" } else { "" };
                    let r = ui.selectable_label(open.as_deref() == Some(name.as_str()), format!("{icon} {leaf}{dirty}"));
                    if r.clicked() {
                        action = Some(FileAction::Open(name.clone(), *kind));
                    }
                    r.context_menu(|ui| {
                        if ui.button("Rename…").clicked() {
                            action = Some(FileAction::Rename(name.clone()));
                            ui.close();
                        }
                        if ui.button("Duplicate").clicked() {
                            action = Some(FileAction::Duplicate(name.clone()));
                            ui.close();
                        }
                        if ui.button("Delete…").clicked() {
                            action = Some(FileAction::Delete(name.clone()));
                            ui.close();
                        }
                        if ui.button("Show in Finder").clicked() {
                            action = Some(FileAction::Reveal(name.clone()));
                            ui.close();
                        }
                    });
                });
            }
        });
        match action {
            Some(FileAction::Open(name, kind)) => self.open_tab(match kind {
                folder::Kind::Sprite => TabKind::Sprite(name),
                folder::Kind::Terrain => TabKind::Terrain(name),
            }),
            Some(FileAction::Rename(name)) => self.modal = Some(Modal::Rename { to: name.clone(), from: name, error: None }),
            Some(FileAction::Duplicate(name)) => self.duplicate(&name),
            Some(FileAction::Delete(name)) => self.modal = Some(Modal::Delete(name)),
            Some(FileAction::Reveal(name)) => self.reveal(&name),
            None => {}
        }
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
                            if let Some(new) = self.insert_color(si, "new", c) {
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
                ui.label(RichText::new(format!("{} px in the project", self.project.color_usage(i))).weak());
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
                    self.remove_color(i, to);
                    self.primary = 0;
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
        let kind = self.kind().clone();
        let fixed = matches!(kind, TabKind::Terrain(_));
        let layers: Vec<(u32, String, bool, bool, bool, &'static str)> = self
            .doc
            .project
            .layers
            .iter()
            .rev()
            .filter(|l| l.visible || !matches!(kind, TabKind::Sandbox) || l.name != "tiles")
            .map(|l| (l.id, l.name.clone(), l.visible, l.locked, l.reference, l.kind_name()))
            .collect();
        for (id, name, visible, locked, reference, lkind) in layers {
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
                    match lkind {
                        "terrain" if !matches!(self.tool, Tool::Terrain | Tool::Hand) => self.tool = Tool::Terrain,
                        "pixels" if self.tool == Tool::Terrain => self.tool = Tool::Pencil,
                        _ => {}
                    }
                }
                let note = match (&kind, reference, lkind) {
                    (TabKind::Sprite(_), true, _) => "not exported".to_string(),
                    (TabKind::Terrain(_), _, "terrain") => "to try the tiles on".to_string(),
                    (_, _, k) => k.to_string(),
                };
                ui.label(RichText::new(note).small().weak());
            });
        }
        if fixed {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            let mut add = |ui: &mut egui::Ui, label: &str, kind: fn(&Project) -> LayerKind| {
                if ui.button(label).clicked() {
                    let mut id = 0;
                    self.edit("Add layer", None, |p| {
                        let k = kind(p);
                        let n = p.layers.len() + 1;
                        let name = (n..).map(|i| format!("layer {i}")).find(|n| p.layer_named(n).is_none()).expect("unbounded");
                        id = p.add_layer(name, k);
                    });
                    self.active_layer = Some(id);
                }
            };
            add(ui, "+ Layer", |_| LayerKind::Pixels(Grid::new()));
            if kind == TabKind::Sandbox {
                add(ui, "+ Terrain layer", |p| LayerKind::Terrain(TerrainLayer { cell_size: p.terrains.first().map_or(p.grid, |_| p.tilesets[0].tile_size), cells: Grid::new() }));
            }
        });
        let Some(id) = self.active_layer else { return };
        let Some(layer) = self.doc.project.layer(id).cloned() else { return };
        if kind == TabKind::Sandbox && layer.name == "tiles" && !layer.visible {
            return;
        }
        ui.horizontal(|ui| {
            let mut name = layer.name.clone();
            if ui.add(egui::TextEdit::singleline(&mut name).desired_width(120.0)).changed() {
                if self.doc.project.layer_named(&name).is_some_and(|l| l.id != id) {
                    self.say(format!("There is already a layer called {name:?}"));
                } else {
                    self.edit("Rename layer", Some(&format!("layer-name-{id}")), |p| p.layer_mut(id).unwrap().name = name);
                }
            }
            if matches!(kind, TabKind::Sprite(_)) {
                let mut export = !layer.reference;
                if ui.checkbox(&mut export, "Exported").on_hover_text("Part of what the game gets; off for sketches and guides").changed() {
                    self.edit("Toggle export", None, |p| {
                        p.layer_mut(id).unwrap().reference = !export;
                        let exported: Vec<u32> = p.layers.iter().filter(|l| !l.reference && l.pixels().is_some()).map(|l| l.id).collect();
                        for s in &mut p.sprites {
                            s.layers = exported.clone();
                        }
                    });
                }
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
            let pixel_layers = self.doc.project.layers.iter().filter(|l| l.pixels().is_some() && l.visible || l.name != "tiles").count();
            if ui.add_enabled(pixel_layers > 1, egui::Button::new("Delete")).clicked() {
                self.edit("Delete layer", None, |p| {
                    p.layers.retain(|l| l.id != id);
                    for s in &mut p.sprites {
                        s.layers.retain(|l| *l != id);
                    }
                });
                self.fix_selection();
            }
        });
    }

    // --- Inspector -----------------------------------------------------------------------------

    fn inspector(&mut self, ui: &mut egui::Ui) {
        match self.kind().clone() {
            TabKind::Sandbox => {
                ui.label(RichText::new("The Sandbox: paint ground with the Terrain tool (T) to try the project's terrains, doodle on the pixel layer. Nothing here is saved.").weak());
                if ui.button("Clear it").clicked() {
                    self.edit("Clear the Sandbox", None, |p| {
                        for l in &mut p.layers {
                            match &mut l.kind {
                                LayerKind::Terrain(t) => t.cells = Grid::new(),
                                LayerKind::Pixels(g) if l.visible || l.name != "tiles" => *g = Grid::new(),
                                _ => {}
                            }
                        }
                    });
                }
            }
            TabKind::Terrain(name) => self.terrain_inspector(ui, &name),
            TabKind::Sprite(name) => {
                let frame = match &self.selected {
                    Selected::Sprite { frame, .. } => *frame,
                    Selected::None => 0,
                };
                self.sprite_inspector(ui, &name, frame);
            }
        }
    }

    fn terrain_inspector(&mut self, ui: &mut egui::Ui, name: &str) {
        let Some(t) = self.project.terrains.get(name).cloned() else { return };
        ui.label(RichText::new(t.preset.label()).strong());
        ui.label(format!("{} tiles of {}×{} px", t.tiles.len(), t.cell, t.cell));
        let mut mirror = matches!(t.preset, folder::Preset::TwoSurface { mirror: true });
        ui.add_enabled(false, egui::Checkbox::new(&mut mirror, "Mirror")).on_disabled_hover_text("Fewer tiles, turned to fit, but no shading: coming later");
        ui.label(RichText::new("Draw each tile in the sheet: a cell of ground shows the tile that matches the air around it, or four quarters cut from the tiles' own corners. Paint the ground below with the Terrain tool (T), right button to erase, to see them together.").small().weak());
        if ui.button("Reset the ground").clicked()
            && let Some(fresh) = workspace::canvas_for(&self.project, self.kind())
        {
            let cells = fresh.layers.iter().find_map(|l| match &l.kind {
                LayerKind::Terrain(t) => Some(t.cells.clone()),
                _ => None,
            });
            if let Some(cells) = cells {
                self.edit("Reset the ground", None, |p| {
                    for l in &mut p.layers {
                        if let LayerKind::Terrain(t) = &mut l.kind {
                            t.cells = cells.clone();
                        }
                    }
                });
            }
        }
    }

    fn sprite_inspector(&mut self, ui: &mut egui::Ui, name: &str, frame: usize) {
        let Some(sprite) = self.doc.project.sprite(name).cloned() else { return };
        let key = |what: &str| format!("{what}-{name}");
        let names: Vec<String> = sprite.tags.get("frames").map(|n| n.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default();

        // Frames.
        ui.horizontal(|ui| {
            ui.label(format!("{} frame(s)", sprite.frames.len()));
            if ui.button("+ Frame").on_hover_text("A blank frame after this one").clicked() {
                self.edit_sprite("Add frame", None, |s| {
                    let f = &s.frames[frame];
                    let mut new = SpriteFrame::new(f.w, f.h);
                    new.duration = if f.duration > 0.0 { f.duration } else { 0.1 };
                    new.points = f.points.clone();
                    if f.duration == 0.0 {
                        for fr in &mut s.frames {
                            fr.duration = 0.1;
                        }
                    }
                    s.frames.insert(frame + 1, new);
                });
                self.selected = Selected::Sprite { name: name.into(), frame: frame + 1 };
            }
            if ui.button("Duplicate").clicked() {
                self.edit_sprite("Duplicate frame", None, |s| {
                    let mut f = s.frames[frame].clone();
                    f.name = None;
                    s.frames.insert(frame + 1, f);
                });
                self.selected = Selected::Sprite { name: name.into(), frame: frame + 1 };
            }
            if sprite.frames.len() > 1 && ui.button("Delete").clicked() {
                self.edit_sprite("Delete frame", None, |s| {
                    s.frames.remove(frame);
                });
                self.selected = Selected::Sprite { name: name.into(), frame: frame.saturating_sub(1) };
            }
        });
        ui.horizontal_wrapped(|ui| {
            for i in 0..sprite.frames.len() {
                let label = names.get(i).filter(|n| !n.is_empty()).map_or(format!("{i}"), |n| format!("{i} {n}"));
                if ui.selectable_label(i == frame, label).clicked() {
                    self.selected = Selected::Sprite { name: name.into(), frame: i };
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.add_enabled(frame > 0, egui::Button::new("◀ Move")).clicked() {
                self.edit_sprite("Move frame", None, |s| s.frames.swap(frame, frame - 1));
                self.selected = Selected::Sprite { name: name.into(), frame: frame - 1 };
            }
            if ui.add_enabled(frame + 1 < sprite.frames.len(), egui::Button::new("Move ▶")).clicked() {
                self.edit_sprite("Move frame", None, |s| s.frames.swap(frame, frame + 1));
                self.selected = Selected::Sprite { name: name.into(), frame: frame + 1 };
            }
        });
        let Some(f) = sprite.frames.get(frame).cloned() else { return };
        egui::Grid::new("frame").num_columns(2).show(ui, |ui| {
            ui.label("Name");
            let mut fname = names.get(frame).cloned().unwrap_or_default();
            if ui.add(egui::TextEdit::singleline(&mut fname).hint_text("for code").desired_width(140.0)).changed() {
                let count = sprite.frames.len();
                self.edit("Frame name", Some(&key("fname")), |p| {
                    let s = sprite_mut(p, name).unwrap();
                    let mut all: Vec<String> = s.tags.get("frames").map(|n| n.split(',').map(|x| x.trim().to_string()).collect()).unwrap_or_default();
                    all.resize(count, String::new());
                    all[frame] = fname.trim().replace(',', " ");
                    if all.iter().all(|n| n.is_empty()) {
                        s.tags.remove("frames");
                    } else {
                        s.tags.insert("frames".into(), all.join(", "));
                    }
                });
            }
            ui.end_row();
            let (mut w, mut h) = (f.rect.w, f.rect.h);
            ui.label("Size");
            let changed = ui
                .horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut w).prefix("w ").range(1..=1024)).changed() | ui.add(egui::DragValue::new(&mut h).prefix("h ").range(1..=1024)).changed()
                })
                .inner;
            if changed {
                self.edit_sprite("Frame size", Some(&key("size")), |s| workspace::resize_frame(&mut s.frames[frame], w, h));
            }
            ui.end_row();
            ui.label("Duration");
            let mut ms = (f.duration * 1000.0).round() as u32;
            if ui.add(egui::DragValue::new(&mut ms).suffix(" ms").range(0..=10_000)).on_hover_text("0 for stills").changed() {
                self.edit("Frame duration", Some(&key("dur")), |p| sprite_mut(p, name).unwrap().frames[frame].duration = ms as f32 / 1000.0);
            }
            ui.end_row();
            ui.label("Exported as");
            let mut t = f.transform;
            if transform_combo(ui, ("frame-t", name, frame), &mut t) {
                self.edit("Frame orientation", None, |p| sprite_mut(p, name).unwrap().frames[frame].transform = t);
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
                    let pn = pname.clone();
                    self.edit("Move point", Some(&key(pname)), |pr| {
                        sprite_mut(pr, name).unwrap().frames[frame].points.insert(pn, v);
                    });
                }
                if ui.small_button("place").clicked() {
                    self.placing_point = Some(pname.clone());
                    self.tool = Tool::Sprite;
                }
                if ui.small_button("all frames").on_hover_text("Copy to every frame").clicked() {
                    let pn = pname.clone();
                    self.edit("Copy point", None, |pr| {
                        for fr in &mut sprite_mut(pr, name).unwrap().frames {
                            fr.points.insert(pn.clone(), v);
                        }
                    });
                }
                if ui.small_button("×").clicked() {
                    let pn = pname.clone();
                    self.edit("Remove point", None, |pr| {
                        sprite_mut(pr, name).unwrap().frames[frame].points.remove(&pn);
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
                let c = IVec2::new(f.rect.w as i32 / 2, f.rect.h as i32 / 2);
                self.edit("Pivot", None, |p| sprite_mut(p, name).unwrap().pivot = has.then_some(c));
            }
            if let Some(mut pv) = sprite.pivot {
                let changed = ui.add(egui::DragValue::new(&mut pv.x).prefix("x ")).changed() | ui.add(egui::DragValue::new(&mut pv.y).prefix("y ")).changed();
                if changed {
                    self.edit("Pivot", Some(&key("pivot")), |p| sprite_mut(p, name).unwrap().pivot = Some(pv));
                }
            }
        });
        for (k, v) in sprite.tags.iter().filter(|(k, _)| *k != "frames") {
            ui.horizontal(|ui| {
                ui.label(RichText::new(k).monospace());
                let mut val = v.clone();
                if ui.text_edit_singleline(&mut val).changed() {
                    let k2 = k.clone();
                    self.edit("Edit tag", Some(&key(k)), |p| {
                        sprite_mut(p, name).unwrap().tags.insert(k2, val);
                    });
                }
            });
        }
    }

    // --- Recolour slots --------------------------------------------------------------

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
}

enum FileAction {
    Open(String, folder::Kind),
    Rename(String),
    Duplicate(String),
    Delete(String),
    Reveal(String),
}
