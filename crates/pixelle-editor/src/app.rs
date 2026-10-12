//! The editor: its state, the window layout, menus, shortcuts, tabs and files.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};
use pixelle_core::folder::{self, File, Preset, ProjectDir, SpriteFile, TerrainFile};
use pixelle_core::palette::presets;
use pixelle_core::render::Indexed;
use pixelle_core::{IRect, IVec2, LayerId, Project, bake, png_io};

use crate::anim::AnimView;
use crate::backend::{CanvasBackend, CpuTiles};
use crate::doc::Document;
use crate::tools::Tool;
use crate::view::View;
use crate::workspace::{self, Disk, TabKind};

const AUTOSAVE_EVERY: Duration = Duration::from_secs(30);
const WATCH_EVERY: Duration = Duration::from_secs(1);

/// What the inspector is showing.
#[derive(Clone, Debug, PartialEq)]
pub enum Selected {
    None,
    Sprite { name: String, frame: usize },
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
    ResizeFrame { frame: usize, orig: IRect },
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
    /// In the animation view: the steps around the one shown, faint.
    pub ghosts: bool,
    pub ghost_count: usize,
    pub ghost_tint: bool,
}

/// A screenshot requested from the command line, for checking the editor without a person.
pub struct Shot {
    pub path: PathBuf,
    pub frames: u32,
    pub requested: bool,
}

/// A tab's canvas and how it was being looked at, while another tab is in front.
pub struct TabState {
    pub doc: Document,
    pub view: View,
    pub active_layer: Option<LayerId>,
    pub selected: Selected,
    pub tool: Tool,
    pub fit_pending: bool,
    pub anim: Option<AnimView>,
}

pub struct Tab {
    pub kind: TabKind,
    /// `None` for the tab in front, whose state lives in the editor itself.
    pub state: Option<TabState>,
}

/// A dialog waiting on the person.
pub enum Modal {
    NewFile { name: String, terrain: bool, w: u32, h: u32, cell: u32, error: Option<String> },
    Rename { from: String, to: String, error: Option<String> },
    Delete(String),
    Recover(BTreeMap<String, String>),
    /// Files changed on disk while there were unsaved changes to them.
    DiskChanged(Vec<String>),
    Close,
}

pub struct Editor {
    /// The project, as files. Open tabs hold newer copies of their own files until written back.
    pub project: ProjectDir,
    /// `None` until a folder is opened: everything stays in memory.
    pub disk: Option<Disk>,
    pub tabs: Vec<Tab>,
    pub active: usize,
    /// Assets changed and not saved whose tabs are closed, and `project` for the root.
    pub unsaved: BTreeSet<String>,
    /// Files to delete on the next save: renamed or deleted assets.
    pub pending_delete: BTreeSet<String>,
    /// The project before the last colour inserted or deleted, while nothing has changed since.
    palette_undo: Option<(String, ProjectDir, BTreeSet<String>)>,

    // The tab in front.
    pub doc: Document,
    pub view: View,
    pub active_layer: Option<LayerId>,
    pub selected: Selected,
    pub tool: Tool,
    pub fit_pending: bool,

    pub backend: Box<dyn CanvasBackend>,
    pub primary: u8,
    pub secondary: u8,
    /// The terrain the terrain brush paints (a terrain layer cell value).
    pub active_terrain: u8,
    pub opts: Options,
    pub selection: Option<IRect>,
    pub floating: Option<Floating>,
    pub clipboard: Option<Indexed>,
    pub drag: Option<Drag>,
    /// Waiting for a click to place this anchor point on the selected frame.
    pub placing_point: Option<String>,
    pub highlight: Option<u8>,
    pub file_filter: String,
    pub clock: Instant,
    pub status: Option<(String, Instant)>,
    pub problems: (u64, Vec<String>),
    pub zoom_acc: f32,
    pub canvas_rect: egui::Rect,
    pub pending_tiles: bool,
    pub modal: Option<Modal>,
    next_key: u64,
    autosave_at: Instant,
    watch_at: Instant,
    allow_close: bool,
    title: String,
    pub shot: Option<Shot>,
    pub new_point_name: String,
    /// The end of a line or rectangle being dragged.
    pub shape_end: Option<IVec2>,
    pub shape_filled: bool,
    /// Where each animation's preview goes, worked out at a revision.
    pub previews: (u64, std::collections::HashMap<String, IRect>),
    pub pending_fit_rect: Option<IRect>,
    /// The open sprite's animation being looked at frame by frame, if one is.
    pub anim: Option<AnimView>,
}

pub struct Args {
    /// The project folder, or its `project.pxl`.
    pub project: Option<PathBuf>,
    /// Open the project opened last, when none is given.
    pub remember: bool,
    pub screenshot: Option<PathBuf>,
    /// An asset to open in a tab.
    pub open: Option<String>,
    /// An animation of that file to open in the animation view.
    pub anim: Option<String>,
    pub zoom: Option<f32>,
    pub tool: Option<String>,
    pub layer: Option<String>,
}

/// Where the last project opened is remembered.
fn last_project_file() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config/pixelle/last-project"))
}

fn folder_of(path: &Path) -> PathBuf {
    if path.file_name().is_some_and(|n| n == folder::ROOT) {
        path.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        path.to_path_buf()
    }
}

pub fn new_project(name: &str, preset: presets::Preset) -> ProjectDir {
    let mut p = ProjectDir::new(name);
    let _ = presets::add(&mut p.palette, preset);
    p
}

impl Editor {
    pub fn new(args: Args) -> anyhow::Result<Self> {
        let project = new_project("untitled", presets::DB32);
        let sandbox = workspace::sandbox_canvas(&project);
        let mut ed = Editor {
            project,
            disk: None,
            tabs: vec![Tab { kind: TabKind::Sandbox, state: None }],
            active: 0,
            unsaved: BTreeSet::new(),
            pending_delete: BTreeSet::new(),
            palette_undo: None,
            doc: Document::new(sandbox),
            view: View::default(),
            active_layer: None,
            selected: Selected::None,
            tool: Tool::Pencil,
            fit_pending: true,
            backend: Box::new(CpuTiles::default()),
            primary: 1,
            secondary: 0,
            active_terrain: 1,
            opts: Options {
                pixel_grid: true,
                tile_grid: true,
                sprite_boxes: true,
                previews: true,
                pixel_perfect: true,
                mirror: false,
                minimap: true,
                ghosts: true,
                ghost_count: 1,
                ghost_tint: true,
            },
            selection: None,
            floating: None,
            clipboard: None,
            drag: None,
            placing_point: None,
            highlight: None,
            file_filter: String::new(),
            clock: Instant::now(),
            status: None,
            problems: (u64::MAX, Vec::new()),
            zoom_acc: 1.0,
            canvas_rect: egui::Rect::NOTHING,
            pending_tiles: false,
            modal: None,
            next_key: 1,
            autosave_at: Instant::now(),
            watch_at: Instant::now(),
            allow_close: false,
            title: String::new(),
            shot: args.screenshot.map(|path| Shot { path, frames: 0, requested: false }),
            new_point_name: String::new(),
            shape_end: None,
            shape_filled: false,
            previews: (u64::MAX, Default::default()),
            pending_fit_rect: None,
            anim: None,
        };
        ed.settle_tab();
        let remember = args.remember;
        let path = args.project.or_else(|| {
            if !remember {
                return None;
            }
            let remembered = std::fs::read_to_string(last_project_file()?).ok()?;
            let p = PathBuf::from(remembered.trim());
            p.join(folder::ROOT).exists().then_some(p)
        });
        if let Some(path) = path {
            ed.open_project(&folder_of(&path), false)?;
        }
        if let Some(name) = args.open {
            let kind = match ed.project.kind(&name) {
                Some(folder::Kind::Sprite) => TabKind::Sprite(name),
                Some(folder::Kind::Terrain) => TabKind::Terrain(name),
                None => anyhow::bail!("no file {name:?} in the project"),
            };
            ed.open_tab(kind);
        }
        if let Some(anim) = args.anim {
            if ed.anim_steps(&anim).is_none_or(|(_, s)| s.is_empty()) {
                anyhow::bail!("no animation {anim:?} in the file open");
            }
            ed.open_anim(&anim);
        }
        if let Some(name) = args.layer {
            ed.active_layer = ed.doc.project.layer_named(&name).map(|l| l.id);
        }
        if let Some(t) = args.tool {
            ed.tool = Tool::ALL
                .into_iter()
                .find(|x| x.name().eq_ignore_ascii_case(&t))
                .ok_or_else(|| anyhow::anyhow!("no tool {t:?}"))?;
        }
        if let Some(z) = args.zoom {
            ed.view.zoom = z;
            ed.fit_pending = false;
        }
        Ok(ed)
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

    pub fn kind(&self) -> &TabKind {
        &self.tabs[self.active].kind
    }

    // --- Projects ------------------------------------------------------------------------------

    /// Open a project folder. One with no `project.pxl` is refused unless `start` says to begin a
    /// project there: opening a folder by mistake must never put a project in it.
    pub fn open_project(&mut self, dir: &Path, start: bool) -> anyhow::Result<()> {
        let project = if dir.join(folder::ROOT).exists() {
            ProjectDir::load(dir)?
        } else if !start {
            anyhow::bail!("{} has no {}: it is not a Pixelle project (File → Open or start a project folder… starts one)", dir.display(), folder::ROOT);
        } else {
            let name = dir.file_name().map_or("untitled".into(), |n| n.to_string_lossy().into_owned());
            let p = new_project(&name, presets::DB32);
            p.save(dir)?;
            p
        };
        let mut disk = Disk::new(dir.to_path_buf());
        disk.note_all(&project);
        let recover = disk.recoverable();
        self.project = project;
        self.disk = Some(disk);
        self.unsaved.clear();
        self.pending_delete.clear();
        self.palette_undo = None;
        self.floating = None;
        self.selection = None;
        self.tabs = vec![Tab { kind: TabKind::Sandbox, state: None }];
        self.active = 0;
        self.doc = Document::new(workspace::sandbox_canvas(&self.project));
        self.view = View::default();
        self.selected = Selected::None;
        self.fit_pending = true;
        self.settle_tab();
        self.primary = if self.project.palette.len() > 1 { 1 } else { 0 };
        self.secondary = 0;
        if !recover.is_empty() {
            self.modal = Some(Modal::Recover(recover));
        }
        if let Some(f) = last_project_file().filter(|_| self.shot.is_none() && !cfg!(test)) {
            let _ = std::fs::create_dir_all(f.parent().unwrap_or(Path::new(".")));
            let _ = std::fs::write(f, dir.display().to_string());
        }
        self.say(format!("Opened {}", dir.display()));
        Ok(())
    }

    pub fn pick_project(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        let Some(dir) = rfd::FileDialog::new().set_title("Open or start a project folder").pick_folder() else { return };
        let start = dir.join(folder::ROOT).exists()
            || rfd::MessageDialog::new()
                .set_title("Start a project here?")
                .set_description(format!("{} has no {}. Start a new Pixelle project in it?", dir.display(), folder::ROOT))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                == rfd::MessageDialogResult::Yes;
        if !start {
            return;
        }
        if let Err(e) = self.open_project(&dir, true) {
            self.say(format!("Could not open {}: {e}", dir.display()));
        }
    }

    fn confirm_discard(&self) -> bool {
        !self.any_unsaved()
            || rfd::MessageDialog::new()
                .set_title("Discard changes?")
                .set_description("The open project has unsaved changes. Discard them?")
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                == rfd::MessageDialogResult::Yes
    }

    pub fn any_unsaved(&self) -> bool {
        !self.unsaved.is_empty() || !self.pending_delete.is_empty() || self.doc.dirty || self.tabs.iter().any(|t| t.state.as_ref().is_some_and(|s| s.doc.dirty))
    }

    /// Whether an asset (or `project`, the root) has changes not on disk.
    pub fn is_unsaved(&self, asset: &str) -> bool {
        self.unsaved.contains(asset)
            || self.tabs.iter().enumerate().any(|(i, t)| {
                t.kind.asset() == Some(asset) && if i == self.active { self.doc.dirty } else { t.state.as_ref().is_some_and(|s| s.doc.dirty) }
            })
    }

    // --- Tabs ----------------------------------------------------------------------------------

    /// The canvas of tab `i`, wherever it is kept.
    fn tab_doc(&self, i: usize) -> &Document {
        if i == self.active { &self.doc } else { &self.tabs[i].state.as_ref().expect("a tab behind keeps its state").doc }
    }

    fn tab_doc_mut(&mut self, i: usize) -> &mut Document {
        if i == self.active { &mut self.doc } else { &mut self.tabs[i].state.as_mut().expect("a tab behind keeps its state").doc }
    }

    /// Read every open tab's canvas back into the project.
    pub fn write_back_all(&mut self) {
        self.commit_floating();
        for i in 0..self.tabs.len() {
            let kind = self.tabs[i].kind.clone();
            if kind == TabKind::Sandbox {
                continue;
            }
            let doc = if i == self.active { &self.doc } else { &self.tabs[i].state.as_ref().expect("kept").doc };
            if doc.dirty {
                let canvas = doc.project.clone();
                workspace::write_back(&mut self.project, &kind, &canvas);
            }
        }
    }

    /// Put the tab in front away, with how it was being looked at.
    fn stash(&mut self) {
        self.commit_floating();
        self.drag = None;
        self.placing_point = None;
        self.selection = None;
        let blank = Document::new(Project::new(""));
        let state = TabState {
            doc: std::mem::replace(&mut self.doc, blank),
            view: self.view,
            active_layer: self.active_layer,
            selected: std::mem::replace(&mut self.selected, Selected::None),
            tool: self.tool,
            fit_pending: self.fit_pending,
            anim: self.anim.take(),
        };
        self.tabs[self.active].state = Some(state);
    }

    /// Bring tab `i` to the front.
    pub fn activate(&mut self, i: usize) {
        if i == self.active || i >= self.tabs.len() {
            return;
        }
        self.stash();
        self.active = i;
        let s = self.tabs[i].state.take().expect("a tab behind keeps its state");
        self.doc = s.doc;
        self.view = s.view;
        self.active_layer = s.active_layer;
        self.selected = s.selected;
        self.tool = s.tool;
        self.fit_pending = s.fit_pending;
        self.anim = s.anim;
        if self.tabs[i].kind == TabKind::Sandbox {
            self.refresh_sandbox();
        }
        self.doc.touch_all();
        self.backend = Box::new(CpuTiles::default());
        self.previews = (u64::MAX, Default::default());
        self.fix_selection();
    }

    /// Open `kind` in a tab, or bring its tab to the front.
    pub fn open_tab(&mut self, kind: TabKind) {
        if let Some(i) = self.tabs.iter().position(|t| t.kind == kind) {
            self.activate(i);
            return;
        }
        let Some(canvas) = workspace::canvas_for(&self.project, &kind) else {
            self.say("No such file");
            return;
        };
        let mut doc = Document::new(canvas);
        doc.clip = kind != TabKind::Sandbox;
        // A file closed with unsaved changes reopens with them still unsaved.
        if let Some(a) = kind.asset()
            && self.unsaved.remove(a)
        {
            doc.dirty = true;
        }
        self.tabs.push(Tab {
            kind,
            state: Some(TabState { doc, view: View::default(), active_layer: None, selected: Selected::None, tool: Tool::Pencil, fit_pending: true, anim: None }),
        });
        let i = self.tabs.len() - 1;
        self.activate(i);
        self.settle_tab();
    }

    /// A sensible layer, tool and selection for a tab just opened.
    fn settle_tab(&mut self) {
        let p = &self.doc.project;
        match self.tabs[self.active].kind.clone() {
            TabKind::Sandbox => {
                self.active_layer = p.layers.iter().find(|l| matches!(l.kind, pixelle_core::layer::LayerKind::Terrain(_))).map(|l| l.id);
                self.tool = if p.terrains.is_empty() { Tool::Pencil } else { Tool::Terrain };
                if p.terrains.is_empty() {
                    self.active_layer = p.layers.iter().rev().find(|l| l.pixels().is_some() && l.visible).map(|l| l.id);
                }
                self.active_terrain = 1;
                let g = self.project.grid.max(1) as i32;
                self.pending_fit_rect = Some(IRect::new(-4 * g, -4 * g, 24 * g as u32, 14 * g as u32));
            }
            TabKind::Sprite(name) => {
                self.active_layer = p.layers.iter().find(|l| l.pixels().is_some() && !l.reference).or(p.layers.first()).map(|l| l.id);
                self.tool = Tool::Pencil;
                self.selected = Selected::Sprite { name, frame: 0 };
            }
            TabKind::Terrain(_) => {
                self.active_layer = p.layers.first().map(|l| l.id);
                self.tool = Tool::Pencil;
                self.active_terrain = 1;
            }
        }
        self.fit_pending = true;
    }

    pub fn close_tab(&mut self, i: usize) {
        if self.tabs[i].kind == TabKind::Sandbox {
            return;
        }
        if i == self.active {
            self.commit_floating();
        }
        let kind = self.tabs[i].kind.clone();
        let doc = self.tab_doc(i);
        if doc.dirty {
            let canvas = doc.project.clone();
            workspace::write_back(&mut self.project, &kind, &canvas);
            if let Some(a) = kind.asset() {
                self.unsaved.insert(a.to_string());
            }
        }
        if i == self.active {
            let to = if i > 0 { i - 1 } else { 0 };
            self.stash();
            self.tabs.remove(i);
            // Nothing is in front for a moment: put the neighbour there.
            self.active = usize::MAX;
            let to = to.min(self.tabs.len() - 1);
            let s = self.tabs[to].state.take().expect("kept");
            self.active = to;
            self.doc = s.doc;
            self.view = s.view;
            self.active_layer = s.active_layer;
            self.selected = s.selected;
            self.tool = s.tool;
            self.fit_pending = s.fit_pending;
            if self.tabs[to].kind == TabKind::Sandbox {
                self.refresh_sandbox();
            }
            self.doc.touch_all();
            self.backend = Box::new(CpuTiles::default());
            self.previews = (u64::MAX, Default::default());
        } else {
            self.tabs.remove(i);
            if i < self.active {
                self.active -= 1;
            }
        }
    }

    fn refresh_sandbox(&mut self) {
        self.write_back_all();
        let i = self.tabs.iter().position(|t| t.kind == TabKind::Sandbox).expect("the Sandbox is always open");
        let project = self.project.clone();
        let doc = self.tab_doc_mut(i);
        workspace::refresh_sandbox(&mut doc.project, &project);
        doc.touch_all();
    }

    /// Rebuild every tab's canvas from the project, after something changed under all of them.
    /// Undo history goes with the old canvases.
    fn rebuild_tabs(&mut self) {
        self.commit_floating();
        for i in 0..self.tabs.len() {
            let kind = self.tabs[i].kind.clone();
            let Some(canvas) = workspace::canvas_for(&self.project, &kind) else { continue };
            let (dirty, layer_name) = {
                let d = self.tab_doc(i);
                let layer = if i == self.active { self.active_layer } else { self.tabs[i].state.as_ref().and_then(|s| s.active_layer) };
                (d.dirty, layer.and_then(|l| d.project.layer(l)).map(|l| l.name.clone()))
            };
            let canvas = if kind == TabKind::Sandbox {
                // Keep what was painted there: it is in no file.
                let mut p = self.tab_doc(i).project.clone();
                workspace::refresh_sandbox(&mut p, &self.project);
                p
            } else {
                canvas
            };
            let mut doc = Document::new(canvas);
            doc.clip = kind != TabKind::Sandbox;
            doc.dirty = dirty;
            let layer = layer_name.and_then(|n| doc.project.layer_named(&n)).map(|l| l.id);
            if i == self.active {
                self.doc = doc;
                self.active_layer = layer.or(self.active_layer);
            } else if let Some(s) = &mut self.tabs[i].state {
                s.doc = doc;
                s.active_layer = layer.or(s.active_layer);
            }
        }
        self.doc.touch_all();
        self.backend = Box::new(CpuTiles::default());
        self.fix_selection();
    }

    // --- The palette and recolour slots, shared by every file ----------------------------------

    /// Carry palette and recolour-slot edits made in the tab in front to the project and every
    /// other tab. Edits that move indices go through [`Editor::insert_color`] and
    /// [`Editor::remove_color`] instead.
    fn share_project_wide(&mut self) {
        let (pal, swaps) = (&self.doc.project.palette, &self.doc.project.swaps);
        if *pal == self.project.palette && *swaps == self.project.swaps {
            return;
        }
        self.project.palette = pal.clone();
        self.project.swaps = swaps.clone();
        let (pal, swaps) = (self.project.palette.clone(), self.project.swaps.clone());
        for (i, t) in self.tabs.iter_mut().enumerate() {
            if i == self.active {
                continue;
            }
            if let Some(s) = &mut t.state {
                s.doc.project.palette = pal.clone();
                s.doc.project.swaps = swaps.clone();
                s.doc.touch_all();
            }
        }
        self.unsaved.insert("project".into());
    }

    /// Put the project's palette in every tab, after something outside the tabs changed it.
    fn broadcast_palette(&mut self) {
        let (pal, swaps) = (self.project.palette.clone(), self.project.swaps.clone());
        for i in 0..self.tabs.len() {
            let d = self.tab_doc_mut(i);
            d.project.palette = pal.clone();
            d.project.swaps = swaps.clone();
            d.touch_all();
        }
    }

    /// Run a change that moves palette indices over the whole project: every file, open or not.
    /// It is one undo step of its own, available until anything else changes.
    fn reindex(&mut self, label: &str, f: impl FnOnce(&mut ProjectDir) -> Vec<String>) {
        self.write_back_all();
        let before = (label.to_string(), self.project.clone(), self.unsaved.clone());
        let dirty: Vec<String> = (0..self.tabs.len()).filter(|&i| self.tab_doc(i).dirty).filter_map(|i| self.tabs[i].kind.asset().map(str::to_string)).collect();
        let changed = f(&mut self.project);
        self.unsaved.extend(changed);
        self.unsaved.extend(dirty);
        self.unsaved.insert("project".into());
        // Open files are rebuilt from the project; whatever they had unsaved is in `unsaved` now.
        for i in 0..self.tabs.len() {
            self.tab_doc_mut(i).dirty = false;
        }
        self.rebuild_tabs();
        for i in 0..self.tabs.len() {
            if let Some(a) = self.tabs[i].kind.asset().map(str::to_string)
                && self.unsaved.remove(&a)
            {
                self.tab_doc_mut(i).dirty = true;
            }
        }
        self.palette_undo = Some(before);
    }

    /// Add a colour at the end of a palette section. Returns its index.
    pub fn insert_color(&mut self, section: usize, name: &str, rgba: [u8; 4]) -> Option<u8> {
        let mut at = None;
        self.reindex("Add colour", |p| {
            let before = p.clone();
            at = p.insert_color(section, name, rgba).ok();
            changed_assets(&before, p)
        });
        at
    }

    /// Delete a colour; its pixels become `replace` (an index as it is now).
    pub fn remove_color(&mut self, index: u8, replace: u8) {
        self.reindex("Delete colour", |p| {
            let before = p.clone();
            p.remove_color(index, replace);
            changed_assets(&before, p)
        });
        self.fix_selection();
    }

    // --- Edits that must not leave a floating selection behind ------------------------------

    /// Change the canvas as one undo step, putting down any floating selection first.
    pub fn edit(&mut self, label: &str, key: Option<&str>, f: impl FnOnce(&mut Project)) {
        self.commit_floating();
        self.doc.edit(label, key, f);
    }

    /// Change the open sprite's file as one undo step — add, remove, reorder or resize frames —
    /// and lay its canvas out again.
    pub fn edit_sprite(&mut self, label: &str, key: Option<&str>, f: impl FnOnce(&mut SpriteFile)) {
        let layer = self.active_layer.and_then(|l| self.doc.project.layer(l)).map(|l| l.name.clone());
        self.edit(label, key, |p| *p = workspace::reshape_sprite(p, f));
        self.active_layer = layer.and_then(|n| self.doc.project.layer_named(&n)).map(|l| l.id).or(self.active_layer);
        self.fix_selection();
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

    /// Whether the last colour insert or delete can still be undone: nothing has changed since.
    pub fn can_undo_palette(&self) -> Option<&str> {
        let quiet = (0..self.tabs.len()).all(|i| !self.tab_doc(i).has_history());
        self.palette_undo.as_ref().filter(|_| quiet).map(|(l, ..)| l.as_str())
    }

    pub fn undo(&mut self) {
        self.commit_floating();
        if !self.doc.has_history()
            && self.can_undo_palette().is_some()
            && let Some((label, project, unsaved)) = self.palette_undo.take()
        {
            self.project = project;
            self.unsaved = unsaved;
            for i in 0..self.tabs.len() {
                if let Some(a) = self.tabs[i].kind.asset().map(str::to_string)
                    && self.unsaved.remove(&a)
                {
                    self.tab_doc_mut(i).dirty = true;
                }
            }
            self.rebuild_tabs();
            self.say(format!("Undid {label}"));
            return;
        }
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
        if self.active_layer.is_none_or(|id| p.layer(id).is_none()) {
            self.active_layer = p.layers.iter().rev().find(|l| l.visible).or(p.layers.last()).map(|l| l.id);
        }
        let ok = match &self.selected {
            Selected::None => true,
            Selected::Sprite { name, frame } => p.sprite(name).is_some_and(|s| *frame < s.frames.len()),
        };
        if !ok {
            self.selected = match p.sprites.iter().find(|s| !workspace::is_anim(s)) {
                Some(s) => Selected::Sprite { name: s.name.clone(), frame: 0 },
                None => Selected::None,
            };
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

    /// Write every changed file, and delete the ones renamed or deleted.
    pub fn save(&mut self) {
        self.write_back_all();
        let Some(disk) = &mut self.disk else {
            self.say("Open a project folder to save into: File → Open project…");
            return;
        };
        let written = match self.project.save(&disk.dir) {
            Ok(w) => w,
            Err(e) => return self.say(format!("Save failed: {e}")),
        };
        let keep: BTreeSet<String> = self.project.to_files().into_keys().collect();
        for rel in std::mem::take(&mut self.pending_delete) {
            if !keep.contains(&rel) {
                let _ = std::fs::remove_file(disk.path(&rel));
            }
        }
        disk.note_all(&self.project);
        disk.clear_autosave();
        self.unsaved.clear();
        for i in 0..self.tabs.len() {
            self.tab_doc_mut(i).dirty = false;
        }
        let n = written.len();
        self.say(if n == 0 { "Nothing to save".to_string() } else { format!("Saved {n} file{}", if n == 1 { "" } else { "s" }) });
    }

    pub fn new_file(&mut self, name: &str, file: File) -> Result<(), String> {
        workspace::valid_name(name)?;
        if self.project.kind(name).is_some() {
            return Err(format!("{name} already exists"));
        }
        let kind = match &file {
            File::Sprite(_) => TabKind::Sprite(name.to_string()),
            File::Terrain(_) => TabKind::Terrain(name.to_string()),
            File::Project(_) => return Err("not a kind of file to make".into()),
        };
        self.project.insert(name, file).map_err(|e| e.to_string())?;
        self.pending_delete.remove(&folder::file_name(name));
        self.unsaved.insert(name.to_string());
        self.unsaved.insert("project".into());
        self.open_tab(kind);
        Ok(())
    }

    pub fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        workspace::valid_name(to)?;
        if from == to {
            return Ok(());
        }
        if self.project.kind(to).is_some() {
            return Err(format!("{to} already exists"));
        }
        self.write_back_all();
        if let Some(s) = self.project.sprites.remove(from) {
            self.project.sprites.insert(to.to_string(), s);
        } else if let Some(t) = self.project.terrains.remove(from) {
            self.project.terrains.insert(to.to_string(), t);
        }
        self.pending_delete.insert(folder::file_name(from));
        self.pending_delete.remove(&folder::file_name(to));
        self.unsaved.remove(from);
        self.unsaved.insert(to.to_string());
        self.unsaved.insert("project".into());
        for i in 0..self.tabs.len() {
            let renamed = match &self.tabs[i].kind {
                TabKind::Sprite(n) if n == from => Some(TabKind::Sprite(to.to_string())),
                TabKind::Terrain(n) if n == from => Some(TabKind::Terrain(to.to_string())),
                _ => None,
            };
            if let Some(k) = renamed {
                self.tabs[i].kind = k;
                let d = self.tab_doc_mut(i);
                for s in &mut d.project.sprites {
                    s.name = match s.name.rsplit_once(folder::ANIM_SEPARATOR) {
                        Some((_, anim)) => format!("{to}{}{anim}", folder::ANIM_SEPARATOR),
                        None => to.to_string(),
                    };
                    if s.tags.contains_key(folder::ANIM_OF) {
                        s.tags.insert(folder::ANIM_OF.into(), to.to_string());
                    }
                }
                for t in &mut d.project.tilesets {
                    t.name = to.to_string();
                }
                for t in &mut d.project.terrains {
                    t.name = to.to_string();
                    t.tileset = to.to_string();
                }
                d.touch_all();
            }
        }
        if let Selected::Sprite { name, .. } = &mut self.selected
            && name == from
        {
            *name = to.to_string();
        }
        Ok(())
    }

    pub fn duplicate(&mut self, name: &str) {
        self.write_back_all();
        let to = workspace::free_name(&self.project, &format!("{name} copy"));
        let file = if let Some(s) = self.project.sprites.get(name) {
            File::Sprite(s.clone())
        } else if let Some(t) = self.project.terrains.get(name) {
            File::Terrain(t.clone())
        } else {
            return;
        };
        if let Err(e) = self.new_file(&to, file) {
            self.say(e);
        }
    }

    pub fn delete(&mut self, name: &str) {
        if let Some(i) = self.tabs.iter().position(|t| t.kind.asset() == Some(name)) {
            self.tab_doc_mut(i).dirty = false;
            self.close_tab(i);
        }
        self.project.remove(name);
        self.unsaved.remove(name);
        self.unsaved.insert("project".into());
        self.pending_delete.insert(folder::file_name(name));
        self.say(format!("Deleted {name}; saving removes its file"));
    }

    pub fn reveal(&mut self, name: &str) {
        let Some(disk) = &self.disk else { return };
        let path = disk.path(&folder::file_name(name));
        let path = if path.exists() { path } else { disk.dir.clone() };
        let r = if cfg!(target_os = "macos") {
            std::process::Command::new("open").arg("-R").arg(&path).spawn()
        } else if cfg!(target_os = "windows") {
            std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn()
        } else {
            std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(&path)).spawn()
        };
        if let Err(e) = r {
            self.say(e.to_string());
        }
    }

    /// Bring outside changes in: files another program wrote, made or deleted.
    fn watch(&mut self) {
        let Some(disk) = &self.disk else { return };
        let changed: Vec<String> = disk.changed().into_iter().filter(|r| !self.pending_delete.contains(r)).collect();
        if changed.is_empty() {
            return;
        }
        let conflicts: Vec<String> = changed
            .iter()
            .filter(|rel| {
                let asset = if *rel == folder::ROOT { "project".to_string() } else { folder::asset_name(rel) };
                self.is_unsaved(&asset)
            })
            .cloned()
            .collect();
        if !conflicts.is_empty() {
            if self.modal.is_none() {
                self.modal = Some(Modal::DiskChanged(conflicts));
            }
            return;
        }
        self.load_from_disk(&changed);
    }

    /// Read `files` again from disk, whatever is in memory.
    fn load_from_disk(&mut self, files: &[String]) {
        let Some(disk) = &self.disk else { return };
        let dir = disk.dir.clone();
        if files.iter().any(|f| f == folder::ROOT) {
            match ProjectDir::load(&dir) {
                Ok(p) => {
                    self.project = p;
                    self.unsaved.clear();
                    for i in 0..self.tabs.len() {
                        self.tab_doc_mut(i).dirty = false;
                    }
                    self.tabs.retain(|t| t.kind.asset().is_none_or(|a| self.project.kind(a).is_some()) || t.state.is_none());
                    self.active = self.active.min(self.tabs.len() - 1);
                    self.rebuild_tabs();
                    self.say("Reloaded the project: it changed on disk");
                }
                Err(e) => self.say(format!("The project changed on disk and does not read: {e}")),
            }
        } else {
            for rel in files {
                let asset = folder::asset_name(rel);
                let path = dir.join(rel);
                if !path.exists() {
                    if let Some(i) = self.tabs.iter().position(|t| t.kind.asset() == Some(&asset)) {
                        self.tab_doc_mut(i).dirty = false;
                        self.close_tab(i);
                    }
                    self.project.remove(&asset);
                    self.unsaved.insert("project".into());
                    continue;
                }
                let read = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| File::from_bytes(rel, &b).map_err(|e| e.to_string()));
                match read {
                    Ok(file) => {
                        let new = self.project.kind(&asset).is_none();
                        if self.project.insert(&asset, file).is_err() {
                            continue;
                        }
                        if new {
                            self.unsaved.insert("project".into());
                        }
                        if let Some(i) = self.tabs.iter().position(|t| t.kind.asset() == Some(&asset)) {
                            let kind = self.tabs[i].kind.clone();
                            if let Some(canvas) = workspace::canvas_for(&self.project, &kind) {
                                let d = self.tab_doc_mut(i);
                                d.edit("Reload from disk", None, |p| *p = canvas);
                                d.dirty = false;
                            }
                        }
                        self.say(format!("Reloaded {rel}: it changed on disk"));
                    }
                    Err(e) => self.say(format!("{rel} changed on disk and does not read: {e}")),
                }
            }
            self.fix_selection();
        }
        if let Some(disk) = &mut self.disk {
            for rel in files {
                disk.note(rel);
            }
        }
    }

    fn autosave(&mut self) {
        if self.disk.is_none() || !self.any_unsaved() {
            return;
        }
        self.write_back_all();
        let mut files = BTreeMap::new();
        let all = self.project.to_files();
        for (rel, text) in all {
            let asset = if rel == folder::ROOT { "project".to_string() } else { folder::asset_name(&rel) };
            if self.is_unsaved(&asset) || (rel == folder::ROOT && !self.pending_delete.is_empty()) {
                files.insert(rel, text);
            }
        }
        if let Some(disk) = &self.disk
            && disk.autosave(&files).is_err()
        {
            self.say("Autosave failed");
        }
    }

    fn recover(&mut self, files: BTreeMap<String, String>) {
        for (rel, text) in files {
            match File::from_text(&rel, &text) {
                Ok(File::Project(m)) => {
                    if let Ok(p) = ProjectDir::from_manifest(m.clone()) {
                        self.project.palette = p.palette;
                        self.project.swaps = p.swaps;
                        self.project.grid = p.grid;
                        // Files the autosaved root lists and the folder lacks were made since.
                        for f in m.files {
                            let a = folder::asset_name(&f);
                            if self.project.kind(&a).is_none() {
                                self.unsaved.insert(a);
                            }
                        }
                        self.unsaved.insert("project".into());
                    }
                }
                Ok(file) => {
                    let a = folder::asset_name(&rel);
                    if self.project.insert(&a, file).is_ok() {
                        self.unsaved.insert(a);
                    }
                }
                Err(e) => self.say(format!("Could not recover {rel}: {e}")),
            }
        }
        self.broadcast_palette();
        self.rebuild_tabs();
        self.say("Recovered unsaved work; save to keep it");
    }

    pub fn import_png(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("PNG", &["png"]).pick_file() else { return };
        let img = match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| png_io::decode(&b).map_err(|e| e.to_string())) {
            Ok(d) => d.to_rgba(),
            Err(e) => return self.say(format!("Could not read {}: {e}", path.display())),
        };
        self.write_back_all();
        let mut report = pixelle_core::import::Report::default();
        let frame_w = if img.w > img.h && img.w % img.h == 0 { img.h } else { img.w };
        match pixelle_core::import::sheet(&mut self.project.palette, &img, frame_w, None, &mut report) {
            Ok(sprite) => {
                self.broadcast_palette();
                let base = path.file_stem().map_or("sprite".into(), |s| pixelle_core::import::snake(&s.to_string_lossy()));
                let name = workspace::free_name(&self.project, &base);
                let frames = sprite.frames.len();
                if let Err(e) = self.new_file(&name, File::Sprite(sprite)) {
                    return self.say(e);
                }
                self.say(format!("Imported {name}: {frames} frame(s), {} new colours", report.added.len()));
            }
            Err(e) => self.say(e),
        }
    }

    pub fn import_aseprite(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("Aseprite", &["aseprite", "ase"]).pick_file() else {
            return;
        };
        let ase = match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| pixelle_core::aseprite::parse(&b).map_err(|e| e.to_string())) {
            Ok(a) => a,
            Err(e) => return self.say(format!("Could not read {}: {e}", path.display())),
        };
        self.write_back_all();
        let base = path.file_stem().map_or("sprite".into(), |s| pixelle_core::import::snake(&s.to_string_lossy()));
        let mut report = pixelle_core::import::Report::default();
        match pixelle_core::import::aseprite(&mut self.project.palette, &ase, &base, true, &mut report) {
            Ok(sprites) => {
                self.broadcast_palette();
                let mut names = Vec::new();
                for (name, s) in sprites {
                    let name = workspace::free_name(&self.project, &name);
                    if let Err(e) = self.new_file(&name, File::Sprite(s)) {
                        return self.say(e);
                    }
                    names.push(name);
                }
                self.say(format!("Imported {}; {} new colours", names.join(", "), report.added.len()));
            }
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

    /// Add a section at the end of the palette. Nothing moves, so it is an ordinary edit.
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
        self.write_back_all();
        let baked = bake::bake_folder(&self.project);
        let img = pixelle_core::render::RgbaImage { w: baked.atlas_width, h: baked.atlas_height, px: baked.atlas_rgba() };
        match png_io::encode_rgba(&img).map_err(|e| e.to_string()).and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string())) {
            Ok(()) => self.say(format!("Exported {}x{} atlas", img.w, img.h)),
            Err(e) => self.say(e),
        }
    }

    pub fn export_baked(&mut self) {
        let name = format!("{}.pxlb", self.project.name);
        let Some(path) = rfd::FileDialog::new().add_filter("Baked", &["pxlb"]).set_file_name(name).save_file() else {
            return;
        };
        self.write_back_all();
        let bytes = bake::bake_folder(&self.project).to_bytes();
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
                egui::CollapsingHeader::new("Files").default_open(true).show(ui, |ui| self.files_panel(ui));
                ui.separator();
                self.tools_panel(ui);
                ui.separator();
                egui::CollapsingHeader::new("Palette").default_open(true).show(ui, |ui| self.palette_panel(ui));
            });
        });
        egui::Panel::right("side").default_size(320.0).show(ui, |ui| self.side_panel(ui));
        if self.anim.is_some() {
            egui::Panel::bottom("anim").show(ui, |ui| self.anim_bar(ui));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::from_gray(38)))
            .show(ui, |ui| self.canvas(ui));

        self.modals(&ctx);
        self.screenshot(&ctx);
    }

    fn housekeeping(&mut self, ctx: &egui::Context) {
        self.share_project_wide();
        self.sync_anim();
        let changes = self.doc.take_changes();
        if !changes.is_empty() {
            self.backend.invalidate(&changes);
        }
        if self.problems.0 != self.doc.revision {
            self.problems = (self.doc.revision, self.doc_problems());
        }
        let title = format!("{}{} — Pixelle", if self.any_unsaved() { "• " } else { "" }, self.project.name);
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        if self.autosave_at.elapsed() > AUTOSAVE_EVERY {
            self.autosave_at = Instant::now();
            self.autosave();
        }
        if self.watch_at.elapsed() > WATCH_EVERY {
            self.watch_at = Instant::now();
            if self.modal.is_none() {
                self.watch();
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.any_unsaved() && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.modal = Some(Modal::Close);
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    /// What is wrong with the open file, as the game would see it.
    fn doc_problems(&self) -> Vec<String> {
        let mut p = ProjectDir::new("");
        p.palette = self.doc.project.palette.clone();
        p.swaps = self.doc.project.swaps.clone();
        match self.kind().clone() {
            TabKind::Sprite(n) => {
                p.sprites.insert(n, workspace::canvas_sprite(&self.doc.project));
            }
            TabKind::Terrain(n) => {
                if let Some(t) = self.project.terrains.get(&n) {
                    p.terrains.insert(n, workspace::canvas_terrain(&self.doc.project, t));
                }
            }
            TabKind::Sandbox => {}
        }
        p.validate()
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.modal.is_some() {
            return;
        }
        let cmd = |k| KeyboardShortcut::new(Modifiers::COMMAND, k);
        let cmd_shift = |k| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, k);
        let consume = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
        if consume(cmd_shift(Key::Z)) || consume(cmd(Key::Y)) {
            self.redo();
        }
        if consume(cmd(Key::Z)) {
            self.undo();
        }
        if consume(cmd(Key::S)) {
            self.save();
        }
        if consume(cmd(Key::O)) {
            self.pick_project();
        }
        if consume(cmd(Key::N)) {
            self.ask_new_file();
        }
        if consume(cmd(Key::W)) {
            self.close_tab(self.active);
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
            let at = self.selection.map(|s| s.min()).unwrap_or_else(|| self.anchor());
            self.paste(at);
        }
        if consume(cmd(Key::A))
            && let Some(b) = self.pixel_layer().and_then(|l| self.doc.grid(l)).and_then(|g| g.bounds())
        {
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
            if self.selection.is_none() && self.floating.is_none() && self.placing_point.is_none() {
                self.close_anim();
            }
            self.commit_floating();
            self.selection = None;
            self.placing_point = None;
            self.drag = None;
        }
        if pressed(Key::Enter) {
            if self.floating.is_none() && self.anim.is_some() {
                self.toggle_play();
            }
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
        // Step through an animation's steps, or a sprite's frames.
        if self.anim.is_some() {
            let step = if pressed(Key::OpenBracket) || pressed(Key::Comma) { -1 } else if pressed(Key::CloseBracket) || pressed(Key::Period) { 1 } else { 0 };
            if step != 0 {
                self.step_anim(step);
            }
        } else if let Selected::Sprite { name, frame } = self.selected.clone() {
            let n = self.doc.project.sprite(&name).map_or(1, |s| s.frames.len()) as i32;
            let step = if pressed(Key::OpenBracket) { -1 } else if pressed(Key::CloseBracket) { 1 } else { 0 };
            if step != 0 {
                self.selected = Selected::Sprite { name, frame: (frame as i32 + step).rem_euclid(n) as usize };
            }
        }
        let grid = self.doc.project.grid.max(1) as i32;
        for (k, d) in [(Key::ArrowLeft, IVec2::NEG_X), (Key::ArrowRight, IVec2::X), (Key::ArrowUp, IVec2::NEG_Y), (Key::ArrowDown, IVec2::Y)] {
            let step = if shift_pressed(k) { Some(d * grid) } else if pressed(k) { Some(d) } else { None };
            if let Some(step) = step
                && self.selection.is_some()
            {
                self.nudge(step);
            }
        }
    }

    fn anchor(&self) -> IVec2 {
        IVec2::new(self.view.center.x.floor() as i32, self.view.center.y.floor() as i32)
    }

    pub fn ask_new_file(&mut self) {
        let g = self.project.grid.max(1);
        self.modal = Some(Modal::NewFile { name: String::new(), terrain: false, w: g * 2, h: g * 2, cell: g, error: None });
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Open or start a project folder…  ⌘O").clicked() {
                    self.pick_project();
                }
                if ui.button("New file…  ⌘N").clicked() {
                    self.ask_new_file();
                }
                if ui.button("Save  ⌘S").on_hover_text("Writes every file that changed").clicked() {
                    self.save();
                }
                if ui.add_enabled(self.kind() != &TabKind::Sandbox, egui::Button::new("Close tab  ⌘W")).clicked() {
                    self.close_tab(self.active);
                }
                ui.separator();
                if ui.button("Import PNG as a sprite…").clicked() {
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
                if ui.button("Bake web file (.pxlb)…").clicked() {
                    self.export_baked();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                let undo = self.doc.can_undo().map(str::to_string).or_else(|| self.can_undo_palette().map(str::to_string));
                let undo = undo.map(|l| format!("Undo {l}  ⌘Z"));
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
                ui.checkbox(&mut self.opts.sprite_boxes, "Frame boxes");
                ui.checkbox(&mut self.opts.previews, "Animation previews");
                ui.checkbox(&mut self.opts.minimap, "Minimap");
                ui.separator();
                if ui.button("Fit everything  ⌘0").clicked() {
                    self.fit_pending = true;
                }
            });
            ui.menu_button("Help", |ui| {
                ui.label("Tools: B pencil · E eraser · L line · R rect · G fill · I picker");
                ui.label("M select · T terrain · S frames · H hand");
                ui.label("X swap colours · [ ] previous/next frame");
                ui.label("Space/middle drag pans · ⌘/pinch scroll zooms");
                ui.label("+/- zoom · ⌘0 fit · Esc deselect · Enter commit");
                ui.label("⌘S saves every changed file · ⌘N new file · ⌘W close tab");
                ui.label("Click an animation's preview to open it frame by frame: [ ] step, Enter plays, Esc goes back");
            });
            ui.separator();
            self.tabs_bar(ui);
        });
    }

    fn tabs_bar(&mut self, ui: &mut egui::Ui) {
        let mut activate = None;
        let mut close = None;
        ui.horizontal(|ui| {
            for (i, t) in self.tabs.iter().enumerate() {
                let dirty = t.kind.asset().is_some_and(|_| if i == self.active { self.doc.dirty } else { t.state.as_ref().is_some_and(|s| s.doc.dirty) });
                let icon = match t.kind {
                    TabKind::Sandbox => "✳",
                    TabKind::Sprite(_) => "🖼",
                    TabKind::Terrain(_) => "▦",
                };
                let label = format!("{icon} {}{}", t.kind.label(), if dirty { " •" } else { "" });
                let r = ui.selectable_label(i == self.active, label);
                let r = match t.kind.asset() {
                    Some(a) => r.on_hover_text(a),
                    None => r.on_hover_text("Try the project's terrains here. Nothing in it is saved."),
                };
                if r.clicked() {
                    activate = Some(i);
                }
                if r.middle_clicked() {
                    close = Some(i);
                }
                if t.kind != TabKind::Sandbox && ui.small_button("×").on_hover_text("Close").clicked() {
                    close = Some(i);
                }
                ui.add_space(6.0);
            }
        });
        if let Some(i) = close {
            self.close_tab(i);
        } else if let Some(i) = activate {
            self.activate(i);
        }
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
        let Some(modal) = self.modal.take() else { return };
        let mut keep = true;
        let mut then: Option<Box<dyn FnOnce(&mut Editor)>> = None;
        let mut modal = modal;
        egui::Modal::new(egui::Id::new("modal")).show(ctx, |ui| match &mut modal {
            Modal::NewFile { name, terrain, w, h, cell, error } => {
                ui.heading("New file");
                let r = ui.add(egui::TextEdit::singleline(name).hint_text("name, e.g. weapons/shotgun").desired_width(260.0));
                if r.gained_focus() || name.is_empty() {
                    r.request_focus();
                }
                ui.horizontal(|ui| {
                    ui.radio_value(terrain, false, "Sprite");
                    ui.radio_value(terrain, true, "Terrain");
                });
                if *terrain {
                    ui.label(Preset::TwoSurface { mirror: false }.label());
                    ui.label(egui::RichText::new("Ground against air, every way a tile faces drawn on its own: lit tops, shaded bottoms.").small().weak());
                    ui.horizontal(|ui| {
                        ui.label("Cell");
                        ui.add(egui::DragValue::new(cell).range(2..=128).suffix(" px"));
                    });
                    ui.add_enabled(false, egui::Checkbox::new(&mut false, "Mirror (fewer tiles, no shading) — coming later"));
                } else {
                    ui.horizontal(|ui| {
                        ui.label("Size");
                        ui.add(egui::DragValue::new(w).range(1..=1024));
                        ui.label("×");
                        ui.add(egui::DragValue::new(h).range(1..=1024));
                    });
                }
                if let Some(e) = error {
                    ui.colored_label(egui::Color32::from_rgb(240, 120, 100), e.as_str());
                }
                ui.horizontal(|ui| {
                    let enter = ui.input(|i| i.key_pressed(Key::Enter));
                    if ui.button("Create").clicked() || enter {
                        let file = if *terrain {
                            let c = (*cell).max(2) & !1;
                            File::Terrain(TerrainFile::new(Preset::TwoSurface { mirror: false }, c))
                        } else {
                            File::Sprite(SpriteFile::new(*w, *h))
                        };
                        let n = name.trim().to_string();
                        let cell_odd = *terrain && *cell % 2 == 1;
                        then = Some(Box::new(move |ed: &mut Editor| {
                            if let Err(e) = ed.new_file(&n, file) {
                                ed.modal = Some(Modal::NewFile { name: n, terrain: false, w: 16, h: 16, cell: 8, error: Some(e) });
                            } else if cell_odd {
                                ed.say("Cells are split in quarters, so the size was made even");
                            }
                        }));
                        keep = false;
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        keep = false;
                    }
                });
            }
            Modal::Rename { from, to, error } => {
                ui.heading(format!("Rename {from}"));
                let r = ui.add(egui::TextEdit::singleline(to).desired_width(260.0));
                r.request_focus();
                if let Some(e) = error {
                    ui.colored_label(egui::Color32::from_rgb(240, 120, 100), e.as_str());
                }
                ui.horizontal(|ui| {
                    if ui.button("Rename").clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
                        let (f, t) = (from.clone(), to.trim().to_string());
                        then = Some(Box::new(move |ed: &mut Editor| {
                            if let Err(e) = ed.rename(&f, &t) {
                                ed.modal = Some(Modal::Rename { from: f, to: t, error: Some(e) });
                            }
                        }));
                        keep = false;
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        keep = false;
                    }
                });
            }
            Modal::Delete(name) => {
                ui.heading(format!("Delete {name}?"));
                ui.label("Its file goes when you next save.");
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        let n = name.clone();
                        then = Some(Box::new(move |ed: &mut Editor| ed.delete(&n)));
                        keep = false;
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            }
            Modal::Recover(files) => {
                ui.heading("Recover unsaved work?");
                ui.label("These were changed and never saved:");
                for rel in files.keys() {
                    ui.label(format!("  {rel}"));
                }
                ui.horizontal(|ui| {
                    if ui.button("Recover").clicked() {
                        let f = std::mem::take(files);
                        then = Some(Box::new(move |ed: &mut Editor| ed.recover(f)));
                        keep = false;
                    }
                    if ui.button("Discard it").clicked() {
                        then = Some(Box::new(|ed: &mut Editor| {
                            if let Some(d) = &ed.disk {
                                d.clear_autosave();
                            }
                        }));
                        keep = false;
                    }
                });
            }
            Modal::DiskChanged(files) => {
                ui.heading("Files changed on disk");
                ui.label("Something else wrote these while you had unsaved changes to them:");
                for rel in files.iter() {
                    ui.label(format!("  {rel}"));
                }
                ui.horizontal(|ui| {
                    if ui.button("Load theirs").clicked() {
                        let f = files.clone();
                        then = Some(Box::new(move |ed: &mut Editor| {
                            for rel in &f {
                                let a = if rel == folder::ROOT { "project".to_string() } else { folder::asset_name(rel) };
                                ed.unsaved.remove(&a);
                                if let Some(i) = ed.tabs.iter().position(|t| t.kind.asset() == Some(&a)) {
                                    ed.tab_doc_mut(i).dirty = false;
                                }
                            }
                            ed.load_from_disk(&f);
                        }));
                        keep = false;
                    }
                    if ui.button("Keep mine").clicked() {
                        let f = files.clone();
                        then = Some(Box::new(move |ed: &mut Editor| {
                            if let Some(d) = &mut ed.disk {
                                for rel in &f {
                                    d.note(rel);
                                }
                            }
                        }));
                        keep = false;
                    }
                });
            }
            Modal::Close => {
                ui.heading("Save changes before closing?");
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        then = Some(Box::new(|ed: &mut Editor| {
                            ed.save();
                            if !ed.any_unsaved() {
                                ed.allow_close = true;
                            }
                        }));
                        keep = false;
                    }
                    if ui.button("Don't save").clicked() {
                        then = Some(Box::new(|ed: &mut Editor| ed.allow_close = true));
                        keep = false;
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            }
        });
        let closing = matches!(modal, Modal::Close);
        if keep {
            self.modal = Some(modal);
        }
        if let Some(f) = then {
            f(self);
        }
        if closing && self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
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

/// The assets whose pixels differ between two versions of a project.
fn changed_assets(before: &ProjectDir, after: &ProjectDir) -> Vec<String> {
    let mut out = Vec::new();
    for (n, s) in &after.sprites {
        if before.sprites.get(n) != Some(s) {
            out.push(n.clone());
        }
    }
    for (n, t) in &after.terrains {
        if before.terrains.get(n) != Some(t) {
            out.push(n.clone());
        }
    }
    out
}

impl eframe::App for Editor {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        crate::colour::tag_srgb(frame);
        self.show(ui);
    }
}
