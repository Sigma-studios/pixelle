//! A canvas being worked on, its per-session undo history, and what changed since the canvas last
//! looked.
//!
//! Every change goes through here. Painting is a *stroke*: cells set one at a time, recorded with
//! their old values, and pushed as one undo step when the stroke ends. Anything else — the
//! palette, layers, sprites, terrains — is an *edit*: a snapshot of the project before it, which
//! undo swaps back in. Snapshots are cheap next to how rarely they happen, and a run of the same
//! edit (dragging a colour slider) is coalesced into one step.
//!
//! Undo lives only as long as the session; nothing of it is written to the file.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use pixelle_core::layer::LayerKind;
use pixelle_core::{Grid, IRect, IVec2, LayerId, Project};

/// How long a run of the same keyed edit keeps folding into one undo step.
const COALESCE: Duration = Duration::from_millis(1500);
/// Undo steps kept.
const HISTORY: usize = 500;

/// What the canvas must redraw.
#[derive(Default, Debug)]
pub struct Changes {
    /// Everything: the palette, visibility, sprites or tiles moved.
    pub all: bool,
    /// Canvas rectangles whose pixels changed.
    pub rects: Vec<IRect>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        !self.all && self.rects.is_empty()
    }

    fn everything(&mut self) {
        self.all = true;
        self.rects.clear();
    }

    fn add(&mut self, r: IRect) {
        if self.all {
            return;
        }
        if let Some(last) = self.rects.last_mut()
            && last.union(&r).w * last.union(&r).h <= 64 * 64
        {
            *last = last.union(&r);
            return;
        }
        self.rects.push(r);
        if self.rects.len() > 256 {
            let all = self.rects.iter().fold(IRect::default(), |a, r| a.union(r));
            self.rects = vec![all];
        }
    }
}

enum Change {
    Cells {
        layer: LayerId,
        label: String,
        /// `(cell, before, after)`.
        cells: Vec<(IVec2, u8, u8)>,
    },
    /// The whole project as it was on the other side of this step.
    Project { label: String, other: Box<Project> },
}

impl Change {
    fn label(&self) -> &str {
        match self {
            Change::Cells { label, .. } | Change::Project { label, .. } => label,
        }
    }
}

struct Stroke {
    layer: LayerId,
    label: String,
    /// Cell → (value before the stroke, value now).
    cells: HashMap<IVec2, (u8, u8)>,
}

pub struct Document {
    pub project: Project,
    /// Changed since it was last read back into its file.
    pub dirty: bool,
    /// Bumped on every change; caches key on it.
    pub revision: u64,
    /// Paint only inside sprite frames and tiles: what is outside them belongs to no file.
    pub clip: bool,
    /// Painting is kept to this rect, when set: the frame the animation view shows.
    pub only: Option<IRect>,
    undo: Vec<Change>,
    redo: Vec<Change>,
    stroke: Option<Stroke>,
    changes: Changes,
    coalesce: Option<(String, Instant)>,
}

impl Document {
    pub fn new(project: Project) -> Self {
        let mut changes = Changes::default();
        changes.everything();
        Self {
            project,
            dirty: false,
            revision: 0,
            clip: false,
            only: None,
            undo: Vec::new(),
            redo: Vec::new(),
            stroke: None,
            changes,
            coalesce: None,
        }
    }

    /// Whether a pixel of a pixel layer belongs to a frame or a tile, when painting is clipped.
    pub fn paintable(&self, p: IVec2) -> bool {
        if let Some(r) = self.only {
            return r.contains(p);
        }
        !self.clip
            || self.project.sprites.iter().any(|s| s.frames.iter().any(|f| f.rect.contains(p)))
            || self.project.tilesets.iter().any(|ts| (0..ts.count).any(|i| ts.tile_rect(i).contains(p)))
    }

    /// Pixels changed outside the history by something that follows from an edit, such as a
    /// terrain's generated cells after their sources: redraw them.
    pub fn touch_rects(&mut self, rects: &[IRect]) {
        self.changes.rects.extend_from_slice(rects);
        self.revision += 1;
    }

    /// Everything must be redrawn: after the canvas was swapped or changed underneath.
    pub fn touch_all(&mut self) {
        self.changes.everything();
        self.revision += 1;
    }

    /// Whether there is anything to undo.
    pub fn has_history(&self) -> bool {
        !self.undo.is_empty() || self.stroke.is_some()
    }

    pub fn take_changes(&mut self) -> Changes {
        std::mem::take(&mut self.changes)
    }

    pub fn can_undo(&self) -> Option<&str> {
        self.undo.last().map(Change::label)
    }

    pub fn can_redo(&self) -> Option<&str> {
        self.redo.last().map(Change::label)
    }

    pub fn grid(&self, layer: LayerId) -> Option<&Grid> {
        match &self.project.layer(layer)?.kind {
            LayerKind::Pixels(g) => Some(g),
            LayerKind::Terrain(t) => Some(&t.cells),
            LayerKind::Instances(_) => None,
        }
    }

    fn grid_mut(project: &mut Project, layer: LayerId) -> Option<&mut Grid> {
        match &mut project.layer_mut(layer)?.kind {
            LayerKind::Pixels(g) => Some(g),
            LayerKind::Terrain(t) => Some(&mut t.cells),
            LayerKind::Instances(_) => None,
        }
    }

    pub fn get(&self, layer: LayerId, p: IVec2) -> u8 {
        self.grid(layer).map_or(0, |g| g.get(p))
    }

    // --- Strokes -------------------------------------------------------------------------------

    pub fn in_stroke(&self) -> bool {
        self.stroke.is_some()
    }

    /// Start painting cells of `layer`. Ends any stroke in progress.
    pub fn begin(&mut self, layer: LayerId, label: &str) {
        self.end();
        self.stroke = Some(Stroke {
            layer,
            label: label.into(),
            cells: HashMap::new(),
        });
    }

    /// Set a cell in the stroke in progress. Does nothing outside a stroke.
    pub fn set(&mut self, p: IVec2, v: u8) {
        let Some(stroke) = &self.stroke else { return };
        let layer = stroke.layer;
        let is_pixels = self.project.layer(layer).is_some_and(|l| l.pixels().is_some());
        if is_pixels && !self.paintable(p) {
            return;
        }
        let Some(stroke) = &mut self.stroke else { return };
        let Some(grid) = Self::grid_mut(&mut self.project, layer) else { return };
        let old = grid.get(p);
        if old == v {
            return;
        }
        grid.set(p, v);
        stroke.cells.entry(p).and_modify(|e| e.1 = v).or_insert((old, v));
        self.note_cells(layer, IRect::new(p.x, p.y, 1, 1));
    }

    /// Put a cell back to how it was before the stroke.
    pub fn revert(&mut self, p: IVec2) {
        let Some(stroke) = &mut self.stroke else { return };
        let Some((old, _)) = stroke.cells.remove(&p) else { return };
        let layer = stroke.layer;
        if let Some(grid) = Self::grid_mut(&mut self.project, layer) {
            grid.set(p, old);
        }
        self.note_cells(layer, IRect::new(p.x, p.y, 1, 1));
    }

    /// Finish the stroke in progress, if any, as one undo step.
    pub fn end(&mut self) {
        let Some(stroke) = self.stroke.take() else { return };
        let mut cells: Vec<(IVec2, u8, u8)> = stroke
            .cells
            .into_iter()
            .filter(|(_, (a, b))| a != b)
            .map(|(p, (a, b))| (p, a, b))
            .collect();
        if cells.is_empty() {
            return;
        }
        cells.sort_by_key(|(p, ..)| (p.y, p.x));
        self.push(Change::Cells {
            layer: stroke.layer,
            label: stroke.label,
            cells,
        });
        self.coalesce = None;
    }

    // --- Edits ---------------------------------------------------------------------------------

    /// Change anything about the project as one undo step. Consecutive edits with the same `key`
    /// within a moment of each other fold into one step.
    pub fn edit(&mut self, label: &str, key: Option<&str>, f: impl FnOnce(&mut Project)) {
        self.end();
        let now = Instant::now();
        let fold = match (key, &self.coalesce) {
            (Some(k), Some((last, at))) => k == last && now.duration_since(*at) < COALESCE,
            _ => false,
        };
        if !fold {
            self.push(Change::Project {
                label: label.into(),
                other: Box::new(self.project.clone()),
            });
        } else {
            self.touch();
        }
        f(&mut self.project);
        self.coalesce = key.map(|k| (k.to_string(), now));
        self.changes.everything();
    }

    fn push(&mut self, change: Change) {
        self.undo.push(change);
        if self.undo.len() > HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.touch();
    }

    fn touch(&mut self) {
        self.dirty = true;
        self.revision += 1;
    }

    pub fn undo(&mut self) -> Option<String> {
        self.end();
        let change = self.undo.pop()?;
        let label = change.label().to_string();
        let back = self.apply(change, true);
        self.redo.push(back);
        self.coalesce = None;
        self.touch();
        Some(label)
    }

    pub fn redo(&mut self) -> Option<String> {
        self.end();
        let change = self.redo.pop()?;
        let label = change.label().to_string();
        let back = self.apply(change, false);
        self.undo.push(back);
        self.coalesce = None;
        self.touch();
        Some(label)
    }

    /// Apply a step (backwards for undo) and return the step that reverses it.
    fn apply(&mut self, change: Change, backwards: bool) -> Change {
        match change {
            Change::Cells { layer, label, cells } => {
                if let Some(grid) = Self::grid_mut(&mut self.project, layer) {
                    for &(p, before, after) in &cells {
                        grid.set(p, if backwards { before } else { after });
                    }
                }
                let bounds = cells
                    .iter()
                    .fold(IRect::default(), |r, (p, ..)| r.union(&IRect::new(p.x, p.y, 1, 1)));
                self.note_cells(layer, bounds);
                Change::Cells { layer, label, cells }
            }
            Change::Project { label, other } => {
                let now = std::mem::replace(&mut self.project, *other);
                self.changes.everything();
                Change::Project {
                    label,
                    other: Box::new(now),
                }
            }
        }
    }

    /// Record that cells in `r` of `layer` changed, widening to everything the change can reach:
    /// a terrain cell changes its neighbours' tiles, and pixels inside a tile or an instanced
    /// sprite show up wherever those are used.
    fn note_cells(&mut self, layer: LayerId, r: IRect) {
        let p = &self.project;
        let Some(l) = p.layer(layer) else { return };
        match &l.kind {
            LayerKind::Terrain(t) => {
                let s = t.cell_size as i32;
                let px = IRect::from_min_max(r.min() * s, r.max() * s).grow(s);
                self.changes.add(px);
            }
            LayerKind::Pixels(_) => {
                let in_tiles = p
                    .tilesets
                    .iter()
                    .any(|ts| ts.layers.contains(&layer) && !ts.bounds().intersect(&r).is_empty());
                let in_instanced = || {
                    p.layers.iter().any(|other| match &other.kind {
                        LayerKind::Instances(list) => list.iter().any(|i| {
                            p.sprite(&i.sprite).is_some_and(|s| {
                                s.layers.contains(&layer)
                                    && s.frames.iter().any(|f| !f.rect.intersect(&r).is_empty())
                            })
                        }),
                        _ => false,
                    })
                };
                if in_tiles || in_instanced() {
                    self.changes.everything();
                } else {
                    self.changes.add(r);
                }
            }
            LayerKind::Instances(_) => self.changes.everything(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> (Document, LayerId) {
        let mut p = Project::new("t");
        p.palette.begin_section("S");
        p.palette.push("a", [1, 1, 1, 255]).unwrap();
        let l = p.add_layer("art", LayerKind::Pixels(Grid::new()));
        (Document::new(p), l)
    }

    #[test]
    fn strokes_undo_as_one_step() {
        let (mut d, l) = doc();
        d.begin(l, "Pencil");
        d.set(IVec2::new(0, 0), 1);
        d.set(IVec2::new(1, 0), 1);
        d.set(IVec2::new(1, 0), 0);
        d.set(IVec2::new(1, 0), 1);
        d.end();
        assert_eq!(d.can_undo(), Some("Pencil"));
        d.undo();
        assert_eq!(d.get(l, IVec2::new(0, 0)), 0);
        assert_eq!(d.get(l, IVec2::new(1, 0)), 0);
        assert!(d.can_undo().is_none());
        d.redo();
        assert_eq!(d.get(l, IVec2::new(1, 0)), 1);
    }

    #[test]
    fn keyed_edits_coalesce() {
        let (mut d, _) = doc();
        for i in 0..5 {
            d.edit("Colour", Some("c1"), |p| p.palette.entry_mut(1).unwrap().rgba[0] = i);
        }
        d.edit("Rename", None, |p| p.name = "x".into());
        assert_eq!(d.can_undo(), Some("Rename"));
        d.undo();
        assert_eq!(d.can_undo(), Some("Colour"));
        d.undo();
        assert_eq!(d.project.palette.rgba(1)[0], 1);
        assert!(d.can_undo().is_none());
    }

    #[test]
    fn reverting_removes_a_cell_from_the_stroke() {
        let (mut d, l) = doc();
        d.begin(l, "Pencil");
        d.set(IVec2::new(3, 3), 1);
        d.revert(IVec2::new(3, 3));
        d.end();
        assert!(d.can_undo().is_none());
        assert_eq!(d.get(l, IVec2::new(3, 3)), 0);
    }
}
