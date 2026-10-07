//! Pixelle art in Bevy.
//!
//! Load a project straight from its folder — `art/project.pxl`, which brings every file it lists
//! along, so saving any of them in the editor hot-reloads the game — or, on the web, the baked
//! file `pixelle-build` writes (`.pxlb`); both give the same [`PixelleArt`]. Refer to sprites by
//! the typed constants `pixelle-build` generates, and let [`PixelleSprite`] keep entities drawing
//! the right frame — through hot reloads, animations and recolouring.
//!
//! ```ignore
//! app.add_plugins(PixellePlugin);
//!
//! let art: Handle<PixelleArt> = assets.load(if cfg!(target_arch = "wasm32") { "art.pxlb" } else { "art/project.pxl" });
//! commands.spawn((
//!     PixelleSprite::new(art.clone(), art::player::POSES)
//!         .frame(art::player::poses::IDLE)
//!         .swap(art::swaps::SHIRT, art::palette::clothing::RUST),
//!     Transform::default(),
//! ));
//! commands.spawn((PixelleSprite::new(art.clone(), art::fx::EXPLOSION), PixelleAnimation::looping()));
//! ```
//!
//! Anchor points come back in the convention sprites are placed by — from the frame's centre, `y`
//! up — see [`FrameRef::offset`].
//!
//! Code that cuts its own rectangles out of sheet images can keep doing so: every sprite is also an
//! image of its frames side by side, labelled `sheet/<name>` — `assets.load("art/project.pxl#sheet/player")`
//! — laid out as a hand-exported sheet would be. See [`sheet_label`].

use std::collections::HashMap;
use std::ops::Range;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext, RenderAssetUsages};
use bevy::image::{ImageSampler, TextureAtlas, TextureAtlasLayout};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use pixelle_core::bake::{self, Baked, BakedFrame, BakedSprite};

pub use pixelle_core;

/// A sprite's name, as `pixelle-build` generates it: `art::weapons::SHOTGUN`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpriteId(pub &'static str);

impl From<SpriteId> for String {
    fn from(id: SpriteId) -> String {
        id.0.to_string()
    }
}

impl AsRef<str> for SpriteId {
    fn as_ref(&self) -> &str {
        self.0
    }
}

/// A terrain's name, as `pixelle-build` generates it: `art::terrains::WALLS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TerrainId(pub &'static str);

impl AsRef<str> for TerrainId {
    fn as_ref(&self) -> &str {
        self.0
    }
}

pub struct PixellePlugin;

impl Plugin for PixellePlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<PixelleArt>()
            .register_asset_loader(PixelleLoader)
            .init_resource::<Variants>()
            .add_systems(Update, (animate, sync_sprites).chain().in_set(PixelleSystems));
    }
}

/// The systems that advance animations and point sprites at their frames. Order your own
/// systems that change [`PixelleSprite`]s before this set to see the change the same frame.
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PixelleSystems;

/// A loaded project or baked file: one atlas, and every sprite in it.
#[derive(Asset, TypePath)]
pub struct PixelleArt {
    /// The whole atlas, in the palette's own colours.
    pub atlas: Handle<Image>,
    /// A layout entry per frame of every sprite.
    pub layout: Handle<TextureAtlasLayout>,
    baked: Baked,
    /// Sprite name → (index in `baked.sprites`, layout index of its first frame).
    by_name: HashMap<String, (usize, usize)>,
}

/// A sprite of a [`PixelleArt`].
#[derive(Clone, Copy)]
pub struct SpriteRef<'a> {
    sprite: &'a BakedSprite,
    first: usize,
}

/// One frame of a sprite.
#[derive(Clone, Copy)]
pub struct FrameRef<'a> {
    frame: &'a BakedFrame,
    /// Its index in the atlas layout.
    pub index: usize,
}

impl PixelleArt {
    pub fn sprite(&self, name: impl AsRef<str>) -> Option<SpriteRef<'_>> {
        let &(i, first) = self.by_name.get(name.as_ref())?;
        Some(SpriteRef { sprite: &self.baked.sprites[i], first })
    }

    /// Every sprite's name, in project order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.baked.sprites.iter().map(|s| s.name.as_str())
    }

    /// A palette entry's colour.
    pub fn color(&self, index: u8) -> Color {
        let [r, g, b, a] = self.baked.palette.get(index as usize).copied().unwrap_or([0; 4]);
        Color::srgba_u8(r, g, b, a)
    }

    /// The palette indices a recolour slot may take.
    pub fn swap_options(&self, slot: impl AsRef<str>) -> Option<Range<u8>> {
        let r = self.baked.swap_options(slot.as_ref())?;
        Some(r.start as u8..r.end as u8)
    }

    /// The atlas recoloured through recolour slots, `(slot, palette index)`: same layout, so any
    /// frame of any sprite can be drawn from it.
    pub fn recoloured(&self, swaps: &[(&str, u8)]) -> Image {
        atlas_image(&self.baked, self.baked.atlas_rgba_swapped(swaps))
    }

    /// A terrain, for autotiling a grid at run time.
    pub fn terrain(&self, name: impl AsRef<str>) -> Option<TerrainRef<'_>> {
        Some(TerrainRef { terrain: self.baked.terrain(name.as_ref())?, atlas: &self.atlas })
    }

    /// The baked data underneath, for anything this API does not cover.
    pub fn baked(&self) -> &Baked {
        &self.baked
    }
}

impl<'a> SpriteRef<'a> {
    pub fn name(&self) -> &'a str {
        &self.sprite.name
    }

    pub fn len(&self) -> usize {
        self.sprite.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sprite.frames.is_empty()
    }

    pub fn is_animation(&self) -> bool {
        self.sprite.is_animation()
    }

    /// Seconds for one pass through every frame.
    pub fn duration(&self) -> f32 {
        self.sprite.frames.iter().map(|f| f.duration).sum()
    }

    pub fn frame(&self, i: usize) -> Option<FrameRef<'a>> {
        Some(FrameRef { frame: self.sprite.frames.get(i)?, index: self.first + i })
    }

    /// Free-form metadata set in the editor.
    pub fn tag(&self, key: &str) -> Option<&'a str> {
        self.sprite.tags.get(key).map(String::as_str)
    }
}

impl FrameRef<'_> {
    pub fn size(&self) -> UVec2 {
        UVec2::new(self.frame.rect.w, self.frame.rect.h)
    }

    pub fn duration(&self) -> f32 {
        self.frame.duration
    }

    /// A named point in frame pixels: `(0, 0)` is the top-left pixel, `y` down.
    pub fn point(&self, name: &str) -> Option<IVec2> {
        self.frame.points.get(name).map(|p| IVec2::new(p.x, p.y))
    }

    /// A named point as an offset from the frame's centre to the middle of that pixel, `y` up —
    /// what to add to a sprite's translation to reach it. Mirror `x` for a flipped sprite.
    pub fn offset(&self, name: &str) -> Option<Vec2> {
        let p = self.point(name)?;
        let s = self.size().as_vec2();
        Some(Vec2::new(p.x as f32 + 0.5 - s.x / 2.0, s.y / 2.0 - (p.y as f32 + 0.5)))
    }

    /// A named rectangle in frame pixels, `y` down.
    pub fn rect(&self, name: &str) -> Option<URect> {
        let r = self.frame.rects.get(name)?;
        Some(URect::new(r.x as u32, r.y as u32, r.x as u32 + r.w, r.y as u32 + r.h))
    }
}

/// A terrain of a [`PixelleArt`].
#[derive(Clone, Copy)]
pub struct TerrainRef<'a> {
    terrain: &'a pixelle_core::bake::BakedTerrain,
    atlas: &'a Handle<Image>,
}

/// One sprite of a terrain cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainPiece {
    /// Where to sample in the atlas, in pixels.
    pub rect: URect,
    /// From the cell's centre to the piece's, `y` up.
    pub offset: Vec2,
    pub size: Vec2,
    pub flip_x: bool,
    pub flip_y: bool,
}

impl TerrainRef<'_> {
    /// The size of a cell in pixels.
    pub fn cell_size(&self) -> u32 {
        self.terrain.tile_size
    }

    /// The pieces that draw a cell, given which neighbours are the same terrain: `same(offset)`
    /// with `offset` in cells, **`y` up**. Answer for cells off the edge of your grid however the
    /// world wants — run-2d calls the floor and the side walls ground.
    pub fn cell(&self, same: impl Fn(IVec2) -> bool) -> Vec<TerrainPiece> {
        let s = self.terrain.tile_size as f32;
        self.terrain
            .cell(|d| same(IVec2::new(d.x, -d.y)))
            .into_iter()
            .map(|p| {
                let r = p.source;
                let at = p.at;
                TerrainPiece {
                    rect: URect::new(r.x as u32, r.y as u32, r.x as u32 + r.w, r.y as u32 + r.h),
                    offset: Vec2::new(at.x as f32 + at.w as f32 / 2.0 - s / 2.0, s / 2.0 - (at.y as f32 + at.h as f32 / 2.0)),
                    size: Vec2::new(at.w as f32, at.h as f32),
                    flip_x: p.flip_x,
                    flip_y: p.flip_y,
                }
            })
            .collect()
    }

    /// A piece as a sprite of the atlas.
    pub fn sprite(&self, piece: &TerrainPiece) -> Sprite {
        Sprite {
            image: self.atlas.clone(),
            rect: Some(Rect::new(
                piece.rect.min.x as f32,
                piece.rect.min.y as f32,
                piece.rect.max.x as f32,
                piece.rect.max.y as f32,
            )),
            custom_size: Some(piece.size),
            flip_x: piece.flip_x,
            flip_y: piece.flip_y,
            ..default()
        }
    }
}

fn atlas_image(baked: &Baked, rgba: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d { width: baked.atlas_width, height: baked.atlas_height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::nearest();
    image
}

#[derive(Default, TypePath)]
pub struct PixelleLoader;

#[derive(Debug, thiserror::Error)]
pub enum PixelleError {
    #[error("could not read the file: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not read {0}: {1}")]
    Read(String, String),
    #[error("{0}")]
    Project(#[from] pixelle_core::folder::Error),
    #[error("{0}")]
    Baked(#[from] bake::Error),
    #[error("{0} is a {1} file; load the project's project.pxl")]
    NotProject(String, &'static str),
}

/// The label of a sprite's sheet image: `art/project.pxl#sheet/player`.
pub fn sheet_label(sprite: impl AsRef<str>) -> String {
    format!("sheet/{}", sprite.as_ref())
}

impl AssetLoader for PixelleLoader {
    type Asset = PixelleArt;
    type Settings = ();
    type Error = PixelleError;

    async fn load(&self, reader: &mut dyn Reader, _: &(), ctx: &mut LoadContext<'_>) -> Result<PixelleArt, PixelleError> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let baked = if bytes.starts_with(bake::MAGIC) {
            Baked::from_bytes(&bytes)?
        } else {
            let path = ctx.path().to_string();
            let root = pixelle_core::folder::File::from_bytes(&path, &bytes)?;
            let pixelle_core::folder::File::Project(manifest) = root else {
                return Err(PixelleError::NotProject(path, root.kind()));
            };
            // Read every file the root lists through the asset server, so each is a dependency:
            // saving any of them reloads the whole project.
            let mut files = HashMap::new();
            for rel in &manifest.files {
                let at = ctx.path().resolve_embed_str(rel).map_err(|e| PixelleError::Read(rel.clone(), e.to_string()))?;
                let bytes = ctx.read_asset_bytes(at).await.map_err(|e| PixelleError::Read(rel.clone(), e.to_string()))?;
                files.insert(rel.clone(), bytes);
            }
            let mut project = pixelle_core::ProjectDir::from_manifest(manifest.clone())?;
            for rel in &manifest.files {
                let file = pixelle_core::folder::File::from_bytes(rel, &files[rel])?;
                project.insert(&pixelle_core::folder::asset_name(rel), file)?;
            }
            for problem in project.validate() {
                warn!("{path}: {problem}");
            }
            bake::bake_folder(&project)
        };
        for s in &baked.sprites {
            if let Some(sheet) = baked.sheet_rgba(&s.name) {
                let mut image = Image::new(
                    Extent3d { width: sheet.w.max(1), height: sheet.h.max(1), depth_or_array_layers: 1 },
                    TextureDimension::D2,
                    if sheet.px.is_empty() { vec![0; 4] } else { sheet.px },
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::default(),
                );
                image.sampler = ImageSampler::nearest();
                ctx.add_labeled_asset(sheet_label(&s.name), image);
            }
        }
        let image = atlas_image(&baked, baked.atlas_rgba());
        let mut layout = TextureAtlasLayout::new_empty(UVec2::new(baked.atlas_width, baked.atlas_height));
        let mut by_name = HashMap::new();
        for (i, s) in baked.sprites.iter().enumerate() {
            let mut first = None;
            for f in &s.frames {
                let r = f.rect;
                let at = layout.add_texture(URect::new(r.x as u32, r.y as u32, r.x as u32 + r.w, r.y as u32 + r.h));
                first.get_or_insert(at);
            }
            by_name.insert(s.name.clone(), (i, first.unwrap_or(0)));
        }
        Ok(PixelleArt {
            atlas: ctx.add_labeled_asset("atlas", image),
            layout: ctx.add_labeled_asset("layout", layout),
            baked,
            by_name,
        })
    }

    fn extensions(&self) -> &[&str] {
        &["pxl", "pxlb"]
    }
}

/// Recoloured atlases already made, so a lobby of players in the same outfit shares one.
#[derive(Resource, Default)]
pub struct Variants {
    made: HashMap<VariantKey, Handle<Image>>,
}

/// Which art, and the recolour slots it was recoloured through (sorted).
type VariantKey = (AssetId<PixelleArt>, Vec<(String, u8)>);

impl Variants {
    /// The atlas of `art` recoloured through `swaps`, made once and then reused.
    pub fn get(&mut self, id: AssetId<PixelleArt>, art: &PixelleArt, swaps: &[(String, u8)], images: &mut Assets<Image>) -> Handle<Image> {
        if swaps.is_empty() {
            return art.atlas.clone();
        }
        let mut key: Vec<(String, u8)> = swaps.to_vec();
        key.sort();
        self.made
            .entry((id, key))
            .or_insert_with(|| {
                let s: Vec<(&str, u8)> = swaps.iter().map(|(n, v)| (n.as_str(), *v)).collect();
                images.add(art.recoloured(&s))
            })
            .clone()
    }

    /// Forget every variant of `id`, e.g. after it reloads with new pixels.
    pub fn forget(&mut self, id: AssetId<PixelleArt>) {
        self.made.retain(|(a, _), _| *a != id);
    }
}

/// Draw a frame of a sprite. Keeps the entity's [`Sprite`] pointing at it, recoloured, whatever
/// happens to the art — including a hot reload that moved everything in the atlas.
#[derive(Component, Clone, Debug)]
#[require(Sprite)]
pub struct PixelleSprite {
    pub art: Handle<PixelleArt>,
    pub name: String,
    pub frame: usize,
    /// Recolour slots: `(slot, palette index)`.
    pub swaps: Vec<(String, u8)>,
    pub flip_x: bool,
    pub flip_y: bool,
}

impl PixelleSprite {
    pub fn new(art: Handle<PixelleArt>, name: impl Into<String>) -> Self {
        Self { art, name: name.into(), frame: 0, swaps: Vec::new(), flip_x: false, flip_y: false }
    }

    pub fn frame(mut self, frame: usize) -> Self {
        self.frame = frame;
        self
    }

    pub fn swap(mut self, slot: &str, index: u8) -> Self {
        self.swaps.retain(|(s, _)| s != slot);
        self.swaps.push((slot.to_string(), index));
        self
    }

    pub fn flipped_x(mut self, flip: bool) -> Self {
        self.flip_x = flip;
        self
    }
}

/// Play a [`PixelleSprite`]'s frames by their durations.
#[derive(Component, Clone, Debug)]
pub struct PixelleAnimation {
    pub elapsed: f32,
    pub speed: f32,
    pub looping: bool,
    pub playing: bool,
}

impl PixelleAnimation {
    pub fn looping() -> Self {
        Self { elapsed: 0.0, speed: 1.0, looping: true, playing: true }
    }

    pub fn once() -> Self {
        Self { looping: false, ..Self::looping() }
    }

    /// Whether a non-looping animation has shown its last frame for its full time.
    pub fn finished(&self, sprite: &SpriteRef) -> bool {
        !self.looping && self.elapsed >= sprite.duration()
    }
}

fn animate(time: Res<Time>, arts: Res<Assets<PixelleArt>>, mut q: Query<(&mut PixelleSprite, &mut PixelleAnimation)>) {
    for (mut ps, mut anim) in &mut q {
        if !anim.playing {
            continue;
        }
        let Some(sprite) = arts.get(&ps.art).and_then(|a| a.sprite(&ps.name)) else { continue };
        let total = sprite.duration();
        if total <= 0.0 {
            continue;
        }
        anim.elapsed += time.delta_secs() * anim.speed;
        let mut t = if anim.looping { anim.elapsed % total } else { anim.elapsed.min(total - 1e-4) };
        let mut frame = sprite.len() - 1;
        for i in 0..sprite.len() {
            let d = sprite.frame(i).map_or(0.0, |f| f.duration());
            if t < d {
                frame = i;
                break;
            }
            t -= d;
        }
        if ps.frame != frame {
            ps.frame = frame;
        }
    }
}

fn sync_sprites(
    mut events: MessageReader<AssetEvent<PixelleArt>>,
    arts: Res<Assets<PixelleArt>>,
    mut images: ResMut<Assets<Image>>,
    mut variants: ResMut<Variants>,
    mut q: Query<(Ref<PixelleSprite>, &mut Sprite)>,
) {
    // Anything whose art (re)loaded must be re-pointed, even if the component did not change.
    let mut reloaded = Vec::new();
    for e in events.read() {
        match e {
            AssetEvent::Added { id } | AssetEvent::LoadedWithDependencies { id } => reloaded.push(*id),
            AssetEvent::Modified { id } => {
                variants.forget(*id);
                reloaded.push(*id);
            }
            _ => {}
        }
    }
    for (ps, mut sprite) in &mut q {
        let fresh = ps.is_changed() || reloaded.contains(&ps.art.id()) || sprite.texture_atlas.is_none();
        if !fresh {
            continue;
        }
        let Some(art) = arts.get(&ps.art) else { continue };
        let Some(frame) = art.sprite(&ps.name).and_then(|s| s.frame(ps.frame)) else {
            warn_once!("no sprite {:?} frame {} in the art", ps.name, ps.frame);
            continue;
        };
        sprite.image = variants.get(ps.art.id(), art, &ps.swaps, &mut images);
        sprite.texture_atlas = Some(TextureAtlas { layout: art.layout.clone(), index: frame.index });
        sprite.flip_x = ps.flip_x;
        sprite.flip_y = ps.flip_y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pixelle_core::IRect;

    #[test]
    fn offsets_match_the_way_run2d_measures_them() {
        // run-2d: muzzle_offset = (MUZZLE.x - 3.5, 3.5 - MUZZLE.y) for an 8x8 frame.
        let f = BakedFrame {
            rect: IRect::new(0, 0, 8, 8),
            duration: 0.0,
            points: [("muzzle".to_string(), pixelle_core::IVec2::new(6, 3))].into(),
            rects: Default::default(),
        };
        let r = FrameRef { frame: &f, index: 0 };
        assert_eq!(r.offset("muzzle"), Some(Vec2::new(2.5, 0.5)));
        assert_eq!(r.point("muzzle"), Some(IVec2::new(6, 3)));
        assert_eq!(r.offset("nope"), None);
    }
}
