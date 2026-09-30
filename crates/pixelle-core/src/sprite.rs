//! Sprites: named exports cut from the canvas.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::d4::D4;
use crate::geom::{IRect, IVec2};
use crate::layer::LayerId;

/// A named, exported piece of the canvas: one frame for a still, several for an animation.
///
/// A sprite composites only the layers it lists, so the same rectangles can export different
/// things — a character from its body layer, and the net thrown over it from the net layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sprite {
    /// Slash-separated path, e.g. `weapons/shotgun`. Unique within a project.
    pub name: String,
    pub layers: Vec<LayerId>,
    pub frames: Vec<Frame>,
    /// The origin the game positions this sprite by, in frame pixels. `None` is the centre.
    #[serde(default)]
    pub pivot: Option<IVec2>,
    /// Free-form metadata for the game.
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub rect: IRect,
    /// Exported with this orientation applied, e.g. a transposed copy of another frame for an
    /// engine that can flip sprites but not transpose them.
    #[serde(default)]
    pub transform: D4,
    /// Seconds this frame shows for in an animation; `0` for stills.
    #[serde(default)]
    pub duration: f32,
    /// Named anchor pixels, in frame coordinates (before `transform`), e.g. `muzzle`.
    #[serde(default)]
    pub points: BTreeMap<String, IVec2>,
    /// Named rectangles, in frame coordinates (before `transform`), e.g. `ink`.
    #[serde(default)]
    pub rects: BTreeMap<String, IRect>,
}

impl Frame {
    pub fn new(rect: IRect) -> Self {
        Self {
            rect,
            transform: D4::IDENTITY,
            duration: 0.0,
            points: BTreeMap::new(),
            rects: BTreeMap::new(),
        }
    }

    pub fn with_duration(mut self, seconds: f32) -> Self {
        self.duration = seconds;
        self
    }

    pub fn with_transform(mut self, transform: D4) -> Self {
        self.transform = transform;
        self
    }

    pub fn with_point(mut self, name: &str, p: IVec2) -> Self {
        self.points.insert(name.into(), p);
        self
    }

    pub fn with_rect(mut self, name: &str, r: IRect) -> Self {
        self.rects.insert(name.into(), r);
        self
    }

    /// The exported size, after `transform`.
    pub fn size(&self) -> (u32, u32) {
        self.transform.size(self.rect.w, self.rect.h)
    }
}

impl Sprite {
    pub fn new(name: impl Into<String>, layers: Vec<LayerId>, frames: Vec<Frame>) -> Self {
        Self {
            name: name.into(),
            layers,
            frames,
            pivot: None,
            tags: BTreeMap::new(),
        }
    }

    pub fn is_animation(&self) -> bool {
        self.frames.len() > 1 && self.frames.iter().any(|f| f.duration > 0.0)
    }

    /// The union of every frame's rectangle on the canvas.
    pub fn bounds(&self) -> IRect {
        self.frames
            .iter()
            .fold(IRect::default(), |acc, f| acc.union(&f.rect))
    }
}
