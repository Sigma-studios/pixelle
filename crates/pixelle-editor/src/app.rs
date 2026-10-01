//! The editor: its state, the window layout, menus, shortcuts and files.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};
use pixelle_core::layer::LayerKind;
use pixelle_core::palette::presets;
use pixelle_core::render::Indexed;
use pixelle_core::{IRect, IVec2, LayerId, Project, bake, pack, png_io};

use crate::backend::{CanvasBackend, CpuTiles};
use crate::doc::{Document, autosave_path};
use crate::tools::Tool;
use crate::view::View;

const AUTOSAVE_EVERY: Duration = Duration::from_secs(30);
const WATCH_EVERY: Duration = Duration::from_secs(1);

/// What the inspector is showing.
#[derive(Clone, Debug, PartialEq)]
pub enum Selected {
    None,
    Sprite { name: String, frame: usize },
    Instance { layer: LayerId, index: usize },
    Terrain(usize),
}

/// Pixels lifted off a layer (or pasted) and not yet put down.
pub struct Floating {
    pub img: Indexed,
    pub pos: IVec2,
    pub layer: LayerId,
    /// Changes whenever `img` does, so its texture is re-uploaded.
    pub key: u64,
}

/// What a drag on the canvas is doing.
#[derive(Clone, Debug)]
pub enum Drag {
    Pan,
    /// Freehand painting: the value painted and the last few pixels, for pixel-perfect elbows.
    Paint { value: u8, recent: Vec<IVec2>, mirror: Option<IRect> },
    Shape { start: IVec2, value: u8 },
    Select { start: IVec2 },
    MoveFloating { grab: IVec2 },
    NewSprite { start: IVec2 },
    MoveFrame { name: String, frame: usize, grab: IVec2, orig: IRect },
    ResizeFrame { name: String, frame: usize, orig: IRect },
    MoveInstance { layer: LayerId, index: usize, grab: IVec2, orig: IVec2 },
    Terrain { value: u8, last: IVec2 },
    Minimap,
}

pub struct Options {
    pub pixel_grid: bool,
    pub tile_grid: bool,
    pub sprite_boxes: bool,
    pub previews: bool,
    pub pixel_perfect: bool,
    pub mirror: bool,
    pub minimap: bool,
}

/// A screenshot requested from the command line, for checking the editor without a person.
pub struct Shot {
    pub path: PathBuf,
    pub frames: u32,
    pub requested: bool,
}

pub struct Editor {
    pub doc: Document,
    pub view: View,
    pub backend: Box<dyn CanvasBackend>,
    pub tool: Tool,
    pub primary: u8,
    pub secondary: u8,
    pub active_layer: Option<LayerId>,
    /// The terrain the terrain brush paints (a terrain layer cell value).
    pub active_terrain: u8,
    pub opts: Options,
    pub selected: Selected,
    pub selection: Option<IRect>,
    pub floating: Option<Floating>,
    pub clipboard: Option<Indexed>,
    pub drag: Option<Drag>,
    /// Waiting for a click to place this anchor point on the selected sprite frame.
    pub placing_point: Option<String>,
    pub highlight: Option<u8>,
    pub sprite_filter: String,
    pub clock: Instant,
    pub status: Option<(String, Instant)>,
    pub problems: (u64, Vec<String>),
    pub fit_pending: bool,
    pub zoom_acc: f32,
    pub canvas_rect: egui::Rect,
    pub pending_tiles: bool,
    next_key: u64,
    autosave_at: Instant,
    watch_at: Instant,
    recover: Option<PathBuf>,
    disk_changed: bool,
    confirm_close: bool,
    allow_close: bool,
    title: String,
    pub shot: Option<Shot>,
    pub new_point_name: String,
    /// The end of a line, rectangle or new sprite being dragged.
    pub shape_end: Option<IVec2>,
    pub shape_filled: bool,
    /// Dragging a new sprite off the grid.
    pub free_drag: bool,
    /// The sprite the instance tool places.
    pub last_sprite: Option<String>,
    /// Where each animation's preview goes, worked out at a revision.
    pub previews: (u64, std::collections::HashMap<String, IRect>),
    pub pending_fit_rect: Option<IRect>,
}

pub struct Args {
    pub file: Option<PathBuf>,
    pub screenshot: Option<PathBuf>,
    pub view: Option<String>,
    pub zoom: Option<f32>,
    pub select: Option<String>,
    pub tool: Option<String>,
    pub layer: Option<String>,
}

impl Editor {
    pub fn new(args: Args) -> anyhow::Result<Self> {
        let (doc, recover) = match &args.file {
            Some(path) if path.exists() => {
                let doc = Document::open(path)?;
                let auto = autosave_path(path);
                let newer = |a: &Path, b: &Path| {
                    let t = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
                    matches!((t(a), t(b)), (Some(x), Some(y)) if x > y)
                };
                let recover = (auto.exists() && newer(&auto, path)).then_some(auto);
                (doc, recover)
            }
            Some(path) => (Document::new(new_project(presets::DB32), Some(path.clone())), None),
            None => (Document::new(new_project(presets::DB32), None), None),
        };
        let mut ed = Editor {
            doc,
            view: View::default(),
            backend: Box::new(CpuTiles::default()),
            tool: Tool::Pencil,
            primary: 1,
            secondary: 0,
            active_layer: None,
            active_terrain: 1,
            opts: Options {
                pixel_grid: true,
                tile_grid: true,
                sprite_boxes: true,
                previews: true,
                pixel_perfect: true,
                mirror: false,
                minimap: true,
            },
            selected: Selected::None,
            selection: None,
            floating: None,
            clipboard: None,
            drag: None,
            placing_point: None,
            highlight: None,
            sprite_filter: String::new(),
            clock: Instant::now(),
            status: None,
            problems: (u64::MAX, Vec::new()),
            fit_pending: true,
            zoom_acc: 1.0,
            canvas_rect: egui::Rect::NOTHING,
            pending_tiles: false,
            next_key: 1,
            autosave_at: Instant::now(),
            watch_at: Instant::now(),
            recover,
            disk_changed: false,
            confirm_close: false,
            allow_close: false,
            title: String::new(),
            shot: args.screenshot.map(|path| Shot { path, frames: 0, requested: false }),
            new_point_name: String::new(),
            shape_end: None,
            shape_filled: false,
            free_drag: false,
            last_sprite: None,
            previews: (u64::MAX, Default::default()),
            pending_fit_rect: None,
        };
        ed.after_load();
        if let Some(name) = args.layer {
            ed.active_layer = ed.doc.project.layer_named(&name).map(|l| l.id);
        }
        if let Some(t) = args.tool {
            ed.tool = Tool::ALL
                .into_iter()
                .find(|x| x.name().eq_ignore_ascii_case(&t))
                .ok_or_else(|| anyhow::anyhow!("no tool {t:?}"))?;
        }
        if let Some(name) = args.select {
            let b = ed.doc.project.sprite(&name).map(|s| s.bounds());
            let b = b.ok_or_else(|| anyhow::anyhow!("no sprite {name:?}"))?;
            ed.view.center = egui::vec2(b.x as f32 + b.w as f32 / 2.0, b.y as f32 + b.h as f32 / 2.0);
            ed.fit_pending = false;
            ed.select_sprite(&name, 0);
        }
        if let Some(v) = args.view {
            let b = ed.doc.project.bookmarks.iter().find(|b| b.name == v);
            let b = b.ok_or_else(|| anyhow::anyhow!("no bookmark {v:?}"))?;
            ed.view.center = egui::vec2(b.rect.x as f32 + b.rect.w as f32 / 2.0, b.rect.y as f32 + b.rect.h as f32 / 2.0);
            ed.fit_pending = args.zoom.is_none();
            if ed.fit_pending {
                ed.pending_fit_rect = Some(b.rect);
            }
        }
        if let Some(z) = args.zoom {
            ed.view.zoom = z;
            ed.fit_pending = false;
        }
        Ok(ed)
    }

    fn after_load(&mut self) {
        let p = &self.doc.project;
        self.active_layer = p
            .layers
            .iter()
            .find(|l| l.pixels().is_some() && !l.reference)
            .or(p.layers.last())
            .map(|l| l.id);
        self.primary = if p.palette.len() > 1 { 1 } else { 0 };
        self.secondary = 0;
        self.selected = Selected::None;
        self.selection = None;
        self.floating = None;
        self.fit_pending = true;
    }

    pub fn key(&mut self) -> u64 {
        self.next_key += 1;
        self.next_key
    }

    pub fn say(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    pub fn time(&self) -> f32 {
        self.clock.elapsed().as_secs_f32()
    }

    // --- Edits that must not leave a floating selection behind ------------------------------

    /// Change the project as one undo step, putting down any floating selection first.
    pub fn edit(&mut self, label: &str, key: Option<&str>, f: impl FnOnce(&mut Project)) {
        self.commit_floating();
        self.doc.edit(label, key, f);
    }

    /// Put floating pixels down where they are.
    pub fn commit_floating(&mut self) {
        let Some(f) = self.floating.take() else { return };
        if !self.doc.in_stroke() {
            self.doc.begin(f.layer, "Move");
        }
        for y in 0..f.img.h {
            for x in 0..f.img.w {
                let v = f.img.get(x, y);
                if v != 0 {
                    self.doc.set(f.pos + IVec2::new(x as i32, y as i32), v);
                }
            }
        }
        self.doc.end();
        self.selection = Some(IRect::new(f.pos.x, f.pos.y, f.img.w, f.img.h));
    }

    /// Lift the selected pixels of the active layer so they can move.
    pub fn lift_selection(&mut self) -> bool {
        if self.floating.is_some() {
            return true;
        }
        let (Some(sel), Some(layer)) = (self.selection, self.pixel_layer()) else {
            return false;
        };
        let mut img = Indexed::new(sel.w, sel.h);
        self.doc.begin(layer, "Move");
        for y in 0..sel.h {
            for x in 0..sel.w {
                let p = IVec2::new(sel.x + x as i32, sel.y + y as i32);
                img.set(x, y, self.doc.get(layer, p));
                self.doc.set(p, 0);
            }
        }
        let key = self.key();
        self.floating = Some(Floating { img, pos: sel.min(), layer, key });
        true
    }

    pub fn copy(&mut self, cut: bool) {
        if let Some(f) = &self.floating {
            self.clipboard = Some(f.img.clone());
            if cut {
                self.floating = None;
                self.doc.end();
                self.selection = None;
            }
            return;
        }
        let (Some(sel), Some(layer)) = (self.selection, self.pixel_layer()) else {
            self.say("Select pixels on a pixel layer first");
            return;
        };
        let mut img = Indexed::new(sel.w, sel.h);
        for y in 0..sel.h {
            for x in 0..sel.w {
                img.set(x, y, self.doc.get(layer, IVec2::new(sel.x + x as i32, sel.y + y as i32)));
            }
        }
        self.clipboard = Some(img);
        if cut {
            self.doc.begin(layer, "Cut");
            for y in 0..sel.h {
                for x in 0..sel.w {
                    self.doc.set(IVec2::new(sel.x + x as i32, sel.y + y as i32), 0);
                }
            }
            self.doc.end();
        }
    }

    pub fn paste(&mut self, at: IVec2) {
        let Some(img) = self.clipboard.clone() else { return };
        let Some(layer) = self.pixel_layer() else {
            self.say("Pick a pixel layer to paste onto");
            return;
        };
        self.commit_floating();
        self.doc.begin(layer, "Paste");
        let key = self.key();
        self.selection = Some(IRect::new(at.x, at.y, img.w, img.h));
        self.floating = Some(Floating { img, pos: at, layer, key });
        self.tool = Tool::Select;
    }

    pub fn delete_selection(&mut self) {
        if self.floating.take().is_some() {
            self.doc.end();
            self.selection = None;
            return;
        }
        if let (Selected::Instance { layer, index }, Tool::Instance) = (&self.selected, self.tool) {
            let (layer, index) = (*layer, *index);
            self.edit("Delete instance", None, |p| {
                if let Some(LayerKind::Instances(list)) = p.layer_mut(layer).map(|l| &mut l.kind)
                    && index < list.len()
                {
                    list.remove(index);
                }
            });
            self.selected = Selected::None;
            return;
        }
        let (Some(sel), Some(layer)) = (self.selection, self.pixel_layer()) else { return };
        self.doc.begin(layer, "Delete");
        for y in 0..sel.h {
            for x in 0..sel.w {
                self.doc.set(IVec2::new(sel.x + x as i32, sel.y + y as i32), 0);
            }
        }
        self.doc.end();
    }

    pub fn transform_floating(&mut self, t: pixelle_core::D4) {
        if !self.lift_selection() {
            return;
        }
        let key = self.key();
        if let Some(f) = &mut self.floating {
            f.img = f.img.transformed(t);
            f.key = key;
            self.selection = Some(IRect::new(f.pos.x, f.pos.y, f.img.w, f.img.h));
        }
    }

    pub fn nudge(&mut self, by: IVec2) {
        if !self.lift_selection() {
            return;
        }
        if let Some(f) = &mut self.floating {
            f.pos += by;
            self.selection = Some(IRect::new(f.pos.x, f.pos.y, f.img.w, f.img.h));
        }
    }

    pub fn undo(&mut self) {
        self.commit_floating();
        match self.doc.undo() {
            Some(l) => self.say(format!("Undid {l}")),
            None => self.say("Nothing to undo"),
        }
        self.fix_selection();
    }

    pub fn redo(&mut self) {
        self.commit_floating();
        match self.doc.redo() {
            Some(l) => self.say(format!("Redid {l}")),
            None => self.say("Nothing to redo"),
        }
        self.fix_selection();
    }

    /// Drop references that an undo or reload may have invalidated.
    pub fn fix_selection(&mut self) {
        let p = &self.doc.project;
        if self.active_layer.is_some_and(|id| p.layer(id).is_none()) {
            self.active_layer = p.layers.last().map(|l| l.id);
        }
        let ok = match &self.selected {
            Selected::None => true,
            Selected::Sprite { name, frame } => p.sprite(name).is_some_and(|s| *frame < s.frames.len()),
            Selected::Instance { layer, index } => {
                matches!(p.layer(*layer).map(|l| &l.kind), Some(LayerKind::Instances(v)) if *index < v.len())
            }
            Selected::Terrain(i) => *i < p.terrains.len(),
        };
        if !ok {
            self.selected = Selected::None;
        }
        if self.primary as usize >= p.palette.len() {
            self.primary = 0;
        }
        if self.secondary as usize >= p.palette.len() {
            self.secondary = 0;
        }
    }

    /// The active layer, if it is an unlocked pixel layer.
    pub fn pixel_layer(&self) -> Option<LayerId> {
        let l = self.doc.project.layer(self.active_layer?)?;
        (l.pixels().is_some() && !l.locked).then_some(l.id)
    }

    // --- Files ---------------------------------------------------------------------------------

    fn confirm_discard(&self) -> bool {
        !self.doc.dirty
            || rfd::MessageDialog::new()
                .set_title("Discard changes?")
                .set_description("The open project has unsaved changes. Discard them?")
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                == rfd::MessageDialogResult::Yes
    }

    pub fn new_file(&mut self, preset: presets::Preset) {
        if !self.confirm_discard() {
            return;
        }
        self.doc = Document::new(new_project(preset), None);
        self.after_load();
    }

    pub fn open(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        if let Some(path) = rfd::FileDialog::new().add_filter("Pixelle", &["pxl"]).pick_file() {
            self.open_path(&path);
        }
    }

    pub fn open_path(&mut self, path: &Path) {
        match Document::open(path) {
            Ok(doc) => {
                self.doc = doc;
                self.after_load();
                self.say(format!("Opened {}", path.display()));
            }
            Err(e) => self.say(format!("Could not open {}: {e}", path.display())),
        }
    }

    pub fn save(&mut self, choose: bool) {
        self.commit_floating();
        if choose || self.doc.path.is_none() {
            let name = format!("{}.pxl", self.doc.project.name);
            let Some(path) = rfd::FileDialog::new().add_filter("Pixelle", &["pxl"]).set_file_name(name).save_file() else {
                return;
            };
            self.doc.path = Some(path);
        }
        match self.doc.save() {
            Ok(()) => self.say("Saved"),
            Err(e) => self.say(format!("Save failed: {e}")),
        }
    }

    fn anchor(&self) -> IVec2 {
        IVec2::new(self.view.center.x.floor() as i32, self.view.center.y.floor() as i32)
    }

    pub fn import_png(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("PNG", &["png"]).pick_file() else { return };
        let Some(layer) = self.pixel_layer() else {
            self.say("Pick a pixel layer to import onto");
            return;
        };
        let img = match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| png_io::decode(&b).map_err(|e| e.to_string())) {
            Ok(d) => d.to_rgba(),
            Err(e) => return self.say(format!("Could not read {}: {e}", path.display())),
        };
        let at = self.anchor();
        let mut report = pixelle_core::import::Report::default();
        let mut err = None;
        self.edit("Import PNG", None, |p| {
            err = pixelle_core::import::paint_rgba(p, layer, at, &img, None, &mut report).err();
        });
        match err {
            Some(e) => self.say(e),
            None => self.say(format!("Imported {}x{}; {} new colours", img.w, img.h, report.added.len())),
        }
    }

    pub fn import_aseprite(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("Aseprite", &["aseprite", "ase"]).pick_file() else {
            return;
        };
        let Some(layer) = self.pixel_layer() else {
            self.say("Pick a pixel layer to import onto");
            return;
        };
        let ase = match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| pixelle_core::aseprite::parse(&b).map_err(|e| e.to_string())) {
            Ok(a) => a,
            Err(e) => return self.say(format!("Could not read {}: {e}", path.display())),
        };
        let name = path.file_stem().map_or("sprite".into(), |s| pixelle_core::import::snake(&s.to_string_lossy()));
        let at = self.anchor();
        let mut report = pixelle_core::import::Report::default();
        let mut result = Ok(Vec::new());
        self.edit("Import Aseprite", None, |p| {
            result = pixelle_core::import::aseprite(p, &ase, at, layer, &name, false, &mut report);
        });
        match result {
            Ok(names) => self.say(format!("Imported {} frames as {}", ase.frames.len(), names.join(", "))),
            Err(e) => self.say(e),
        }
    }

    pub fn import_palette(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("Palette", &["hex", "gpl"]).pick_file() else { return };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => return self.say(e.to_string()),
        };
        let parsed = if path.extension().is_some_and(|e| e == "gpl") {
            pixelle_core::import::parse_gpl(&text)
        } else {
            pixelle_core::import::parse_hex(&text)
        };
        let colours = match parsed {
            Ok(c) => c,
            Err(e) => return self.say(e),
        };
        let label = path.file_stem().map_or("Palette".into(), |s| s.to_string_lossy().into_owned());
        self.add_palette_section(&label, &colours);
    }

    pub fn add_palette_section(&mut self, label: &str, colours: &[(String, [u8; 4])]) {
        if self.doc.project.palette.len() + colours.len() > 256 {
            self.say("That does not fit: a palette holds 256 entries");
            return;
        }
        self.edit("Add palette section", None, |p| {
            p.palette.begin_section(label);
            for (n, c) in colours {
                let _ = p.palette.push(n.clone(), *c);
            }
        });
    }

    pub fn export_atlas(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("PNG", &["png"]).set_file_name("atlas.png").save_file() else {
            return;
        };
        let atlas = pack::pack(&self.doc.project, 1);
        let png = png_io::encode_rgba(&atlas.image().to_rgba(&self.doc.project.palette));
        match png.map_err(|e| e.to_string()).and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string())) {
            Ok(()) => self.say(format!("Exported {}x{} atlas", atlas.width, atlas.height)),
            Err(e) => self.say(e),
        }
    }

    pub fn export_baked(&mut self) {
        let name = format!("{}.pxlb", self.doc.project.name);
        let Some(path) = rfd::FileDialog::new().add_filter("Baked", &["pxlb"]).set_file_name(name).save_file() else {
            return;
        };
        let bytes = bake::bake(&self.doc.project).to_bytes();
        match std::fs::write(&path, &bytes) {
            Ok(()) => self.say(format!("Baked {} bytes", bytes.len())),
            Err(e) => self.say(e.to_string()),
        }
    }

    // --- The frame -----------------------------------------------------------------------------

    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.housekeeping(&ctx);
        self.shortcuts(&ctx);

        egui::Panel::top("menu").show(ui, |ui| self.menu(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("tools").default_size(250.0).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("left").auto_shrink(false).show(ui, |ui| {
                self.tools_panel(ui);
                ui.separator();
                egui::CollapsingHeader::new("Palette").default_open(true).show(ui, |ui| self.palette_panel(ui));
            });
        });
        egui::Panel::right("side").default_size(320.0).show(ui, |ui| self.side_panel(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::from_gray(38)))
            .show(ui, |ui| self.canvas(ui));

        self.modals(&ctx);
        self.screenshot(&ctx);
    }

    fn housekeeping(&mut self, ctx: &egui::Context) {
        let changes = self.doc.take_changes();
        if !changes.is_empty() {
            self.backend.invalidate(&changes);
        }
        if self.problems.0 != self.doc.revision {
            self.problems = (self.doc.revision, self.doc.project.validate());
        }
        let title = format!(
            "{}{} — Pixelle",
            if self.doc.dirty { "• " } else { "" },
            self.doc.path.as_ref().and_then(|p| p.file_name()).map_or("untitled".into(), |n| n.to_string_lossy().into_owned())
        );
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        if self.autosave_at.elapsed() > AUTOSAVE_EVERY {
            self.autosave_at = Instant::now();
            if self.doc.dirty && self.doc.path.is_some() && self.doc.autosave().is_err() {
                self.say("Autosave failed");
            }
        }
        if self.watch_at.elapsed() > WATCH_EVERY {
            self.watch_at = Instant::now();
            if !self.disk_changed && self.doc.changed_on_disk() {
                if self.doc.dirty {
                    self.disk_changed = true;
                } else if self.doc.reload().is_ok() {
                    self.fix_selection();
                    self.say("Reloaded: the file changed on disk");
                }
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.doc.dirty && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close = true;
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let cmd = |k| KeyboardShortcut::new(Modifiers::COMMAND, k);
        let cmd_shift = |k| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, k);
        let consume = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
        if consume(cmd_shift(Key::Z)) || consume(cmd(Key::Y)) {
            self.redo();
        }
        if consume(cmd(Key::Z)) {
            self.undo();
        }
        if consume(cmd_shift(Key::S)) {
            self.save(true);
        }
        if consume(cmd(Key::S)) {
            self.save(false);
        }
        if consume(cmd(Key::O)) {
            self.open();
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        if consume(cmd(Key::C)) {
            self.copy(false);
        }
        if consume(cmd(Key::X)) {
            self.copy(true);
        }
        if consume(cmd(Key::V)) {
            let at = self
                .selection
                .map(|s| s.min())
                .unwrap_or_else(|| IVec2::new(self.view.center.x as i32, self.view.center.y as i32));
            self.paste(at);
        }
        if consume(cmd(Key::A))
            && let Some(b) = self.pixel_layer().and_then(|l| self.doc.grid(l)).and_then(|g| g.bounds()) {
                self.commit_floating();
                self.selection = Some(b);
                self.tool = Tool::Select;
            }
        if consume(cmd(Key::Num0)) {
            self.fit_pending = true;
        }

        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k) && i.modifiers.is_none());
        let shift_pressed = |k: Key| ctx.input(|i| i.key_pressed(k) && i.modifiers.shift_only());
        for tool in Tool::ALL {
            let key = Key::from_name(tool.key()).expect("tool keys are letters");
            if pressed(key) {
                if self.tool != tool {
                    self.commit_floating();
                }
                self.tool = tool;
            }
        }
        if pressed(Key::X) {
            std::mem::swap(&mut self.primary, &mut self.secondary);
        }
        if pressed(Key::Escape) {
            self.commit_floating();
            self.selection = None;
            self.placing_point = None;
            self.drag = None;
        }
        if pressed(Key::Enter) {
            self.commit_floating();
        }
        if pressed(Key::Delete) || pressed(Key::Backspace) {
            self.delete_selection();
        }
        if pressed(Key::Plus) || pressed(Key::Equals) {
            let c = self.canvas_rect.center();
            self.view.step_zoom(self.canvas_rect, c, 1);
        }
        if pressed(Key::Minus) {
            let c = self.canvas_rect.center();
            self.view.step_zoom(self.canvas_rect, c, -1);
        }
        let grid = self.doc.project.grid.max(1) as i32;
        for (k, d) in [(Key::ArrowLeft, IVec2::NEG_X), (Key::ArrowRight, IVec2::X), (Key::ArrowUp, IVec2::NEG_Y), (Key::ArrowDown, IVec2::Y)] {
            let step = if shift_pressed(k) { Some(d * grid) } else if pressed(k) { Some(d) } else { None };
            if let Some(step) = step {
                match (&self.selected, self.tool) {
                    (Selected::Instance { layer, index }, Tool::Instance) => {
                        let (layer, index) = (*layer, *index);
                        self.edit("Move instance", Some("nudge"), |p| {
                            if let Some(LayerKind::Instances(list)) = p.layer_mut(layer).map(|l| &mut l.kind)
                                && let Some(i) = list.get_mut(index)
                            {
                                i.pos += step;
                            }
                        });
                    }
                    _ if self.selection.is_some() => self.nudge(step),
                    _ => {}
                }
            }
        }
        if self.tool == Tool::Instance
            && let Selected::Instance { layer, index } = self.selected.clone()
        {
            let flip = if pressed(Key::F) {
                Some(pixelle_core::D4::FLIP_X)
            } else if shift_pressed(Key::F) {
                Some(pixelle_core::D4::FLIP_Y)
            } else {
                None
            };
            let frame_step = if pressed(Key::OpenBracket) { -1 } else if pressed(Key::CloseBracket) { 1 } else { 0 };
            if flip.is_some() || frame_step != 0 {
                self.edit("Change instance", None, |p| {
                    let frames = |name: &str| p.sprite(name).map_or(1, |s| s.frames.len() as i32);
                    let count = match p.layer(layer).map(|l| &l.kind) {
                        Some(LayerKind::Instances(list)) => list.get(index).map(|i| frames(&i.sprite)),
                        _ => None,
                    };
                    if let Some(LayerKind::Instances(list)) = p.layer_mut(layer).map(|l| &mut l.kind)
                        && let Some(i) = list.get_mut(index)
                    {
                        if let Some(f) = flip {
                            i.transform = f.after(i.transform);
                        }
                        let n = count.unwrap_or(1);
                        i.frame = (i.frame as i32 + frame_step).rem_euclid(n) as u32;
                    }
                });
            }
        }
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                ui.menu_button("New", |ui| {
                    for preset in presets::ALL {
                        if ui.button(format!("With {}", preset.0)).clicked() {
                            self.new_file(preset);
                        }
                    }
                });
                if ui.button("Open…  ⌘O").clicked() {
                    self.open();
                }
                if ui.button("Save  ⌘S").clicked() {
                    self.save(false);
                }
                if ui.button("Save as…  ⇧⌘S").clicked() {
                    self.save(true);
                }
                ui.separator();
                if ui.button("Import PNG onto layer…").clicked() {
                    self.import_png();
                }
                if ui.button("Import Aseprite…").clicked() {
                    self.import_aseprite();
                }
                if ui.button("Import palette (.hex, .gpl)…").clicked() {
                    self.import_palette();
                }
                ui.separator();
                if ui.button("Export atlas PNG…").clicked() {
                    self.export_atlas();
                }
                if ui.button("Bake runtime file (.pxlb)…").clicked() {
                    self.export_baked();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                let undo = self.doc.can_undo().map(|l| format!("Undo {l}  ⌘Z"));
                if ui.add_enabled(undo.is_some(), egui::Button::new(undo.unwrap_or("Undo".into()))).clicked() {
                    self.undo();
                }
                let redo = self.doc.can_redo().map(|l| format!("Redo {l}  ⇧⌘Z"));
                if ui.add_enabled(redo.is_some(), egui::Button::new(redo.unwrap_or("Redo".into()))).clicked() {
                    self.redo();
                }
                ui.separator();
                if ui.button("Copy  ⌘C").clicked() {
                    self.copy(false);
                }
                if ui.button("Cut  ⌘X").clicked() {
                    self.copy(true);
                }
                if ui.button("Paste  ⌘V").clicked() {
                    let at = self.selection.map(|s| s.min()).unwrap_or(self.anchor());
                    self.paste(at);
                }
                if ui.button("Delete  ⌫").clicked() {
                    self.delete_selection();
                }
                if ui.button("Deselect  Esc").clicked() {
                    self.commit_floating();
                    self.selection = None;
                }
            });
            ui.menu_button("View", |ui| {
                ui.checkbox(&mut self.opts.pixel_grid, "Pixel grid");
                ui.checkbox(&mut self.opts.tile_grid, "Tile grid");
                ui.checkbox(&mut self.opts.sprite_boxes, "Sprite boxes");
                ui.checkbox(&mut self.opts.previews, "Animation previews");
                ui.checkbox(&mut self.opts.minimap, "Minimap");
                ui.separator();
                if ui.button("Fit everything  ⌘0").clicked() {
                    self.fit_pending = true;
                }
                ui.menu_button("Go to", |ui| {
                    let marks = self.doc.project.bookmarks.clone();
                    for b in marks {
                        if ui.button(&b.name).clicked() {
                            self.view.fit(self.canvas_rect, b.rect);
                        }
                    }
                });
            });
            ui.menu_button("Help", |ui| {
                ui.label("Tools: B pencil · E eraser · L line · R rect · G fill · I picker");
                ui.label("M select · T terrain · S sprite · N instance · H hand");
                ui.label("X swap colours · Space/middle drag pans · ⌘/pinch scroll zooms");
                ui.label("+/- zoom · ⌘0 fit · Esc deselect · Enter commit");
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(format!("{}×", self.view.zoom));
            if let Some(p) = ui.ctx().pointer_hover_pos().filter(|p| self.canvas_rect.contains(*p)) {
                let px = self.view.pixel(self.canvas_rect, p);
                let idx = self.active_layer.map_or(0, |l| self.doc.get(l, px));
                let name = self.doc.project.palette.entry(idx).map_or("", |e| e.name.as_str());
                ui.label(format!("{}, {}  ·  {idx} {name}", px.x, px.y));
            }
            if let Some(l) = self.active_layer.and_then(|id| self.doc.project.layer(id)) {
                ui.label(format!("layer {} ({})", l.name, l.kind_name()));
            }
            ui.label(self.tool.hint());
            if let Some((msg, at)) = &self.status
                && at.elapsed() < Duration::from_secs(6)
            {
                ui.colored_label(egui::Color32::LIGHT_YELLOW, msg);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let n = self.problems.1.len();
                if n > 0 {
                    ui.colored_label(egui::Color32::from_rgb(240, 150, 80), format!("⚠ {n} problem(s)"))
                        .on_hover_text(self.problems.1.join("\n"));
                }
            });
        });
    }

    fn modals(&mut self, ctx: &egui::Context) {
        if let Some(auto) = self.recover.clone() {
            egui::Modal::new(egui::Id::new("recover")).show(ctx, |ui| {
                ui.heading("Recover unsaved changes?");
                ui.label("An autosave newer than this file was found. It holds changes that were never saved.");
                ui.horizontal(|ui| {
                    if ui.button("Recover").clicked() {
                        match pixelle_core::format::load_file(&auto) {
                            Ok(p) => {
                                self.doc.edit("Recover autosave", None, |proj| *proj = p);
                                self.say("Recovered; save to keep it");
                            }
                            Err(e) => self.say(format!("The autosave is unreadable: {e}")),
                        }
                        self.recover = None;
                    }
                    if ui.button("Discard it").clicked() {
                        let _ = std::fs::remove_file(&auto);
                        self.recover = None;
                    }
                });
            });
        }
        if self.disk_changed {
            egui::Modal::new(egui::Id::new("disk")).show(ctx, |ui| {
                ui.heading("The file changed on disk");
                ui.label("Something else wrote the project while you had unsaved changes.");
                ui.horizontal(|ui| {
                    if ui.button("Load theirs").clicked() {
                        let _ = self.doc.reload();
                        self.fix_selection();
                        self.disk_changed = false;
                    }
                    if ui.button("Keep mine").clicked() {
                        self.doc.disk_mtime = self
                            .doc
                            .path
                            .as_ref()
                            .and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
                        self.disk_changed = false;
                    }
                });
            });
        }
        if self.confirm_close {
            egui::Modal::new(egui::Id::new("close")).show(ctx, |ui| {
                ui.heading("Save changes before closing?");
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        self.save(false);
                        if !self.doc.dirty {
                            self.allow_close = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        self.confirm_close = false;
                    }
                    if ui.button("Don't save").clicked() {
                        self.allow_close = true;
                        self.confirm_close = false;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_close = false;
                    }
                });
            });
        }
    }

    fn screenshot(&mut self, ctx: &egui::Context) {
        let Some(shot) = &mut self.shot else { return };
        shot.frames += 1;
        ctx.request_repaint();
        if !shot.requested && shot.frames > 12 && !self.pending_tiles {
            shot.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let img = pixelle_core::render::RgbaImage {
                w: image.size[0] as u32,
                h: image.size[1] as u32,
                px: image.pixels.iter().flat_map(|c| c.to_array()).collect(),
            };
            let path = shot.path.clone();
            match png_io::encode_rgba(&img).map_err(|e| e.to_string()).and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string())) {
                Ok(()) => println!("screenshot: {}", path.display()),
                Err(e) => eprintln!("screenshot failed: {e}"),
            }
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

pub fn new_project(preset: presets::Preset) -> Project {
    let mut p = Project::new("untitled");
    let _ = presets::add(&mut p.palette, preset);
    p.add_layer("Art", LayerKind::Pixels(Default::default()));
    p
}

impl eframe::App for Editor {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        crate::colour::tag_srgb(frame);
        self.show(ui);
    }
}
