//! run-2d's art, straight from `run2d.pxl`.
//!
//! Everything here is looked up by name through the generated `art` module — no frame indices,
//! no pixel constants. Weapons sit in hands by the file's own `hand` and `trigger` points; players
//! are recoloured through its recolour slots; the walls use the transposed frames the file bakes.
//!
//! `cargo run -p showcase` loads the project (edit it in the editor and it reloads);
//! `cargo run -p showcase --release` loads the baked `run2d.pxlb`.
//! `PIXELLE_FILE=run2d.pxlb` picks the file; `PIXELLE_SCREENSHOT=out.png` saves a screenshot and
//! quits.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy_pixelle::{PixelleAnimation, PixelleArt, PixellePlugin, PixelleSprite, PixelleSystems};

include!(concat!(env!("OUT_DIR"), "/art.rs"));

use art::palette;

const SCALE: f32 = 5.0;

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(ImagePlugin::default_nearest())
                .set(AssetPlugin { watch_for_changes_override: Some(cfg!(debug_assertions)), ..default() })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Pixelle showcase".into(),
                        resolution: (1280, 720).into(),
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_plugins(PixellePlugin)
        .add_systems(Startup, setup)
        .add_systems(Update, (populate, seat_held.after(PixelleSystems), screenshot))
        .run();
}

#[derive(Resource)]
struct Art(Handle<PixelleArt>);

/// Something held by the entity `by`: seated with the holder's `hand` on this sprite's `trigger`.
#[derive(Component)]
struct Held {
    by: Entity,
}

fn setup(mut commands: Commands, assets: Res<AssetServer>) {
    let file = std::env::var("PIXELLE_FILE")
        .unwrap_or_else(|_| if cfg!(debug_assertions) { "run2d.pxl" } else { "run2d.pxlb" }.into());
    commands.insert_resource(Art(assets.load(file)));
    commands.spawn((
        Camera2d,
        Msaa::Off,
        Projection::Orthographic(OrthographicProjection { scale: 1.0 / SCALE, ..OrthographicProjection::default_2d() }),
    ));
}

fn at(x: f32, y: f32, z: f32) -> Transform {
    Transform::from_xyz(x, y, z)
}

fn populate(mut commands: Commands, art: Res<Art>, arts: Res<Assets<PixelleArt>>, mut done: Local<bool>) {
    let Some(a) = arts.get(&art.0) else { return };
    if *done {
        return;
    }
    *done = true;
    let h = &art.0;
    let sprite = |name: bevy_pixelle::SpriteId| PixelleSprite::new(h.clone(), name);
    commands.insert_resource(ClearColor(a.color(palette::world::SKY)));

    // A lobby: every clothing colour on a shirt, skin tones in turn.
    let clothing = a.swap_options(art::swaps::SHIRT).expect("the shirt slot picks from Clothing");
    let skins = a.swap_options(art::swaps::SKIN).expect("skin picks from Skin tones");
    for (i, shirt) in clothing.clone().enumerate() {
        let hair = clothing.start + (i as u8 * 5) % clothing.len() as u8;
        let skin = skins.start + i as u8 % skins.len() as u8;
        commands.spawn((
            sprite(art::player::POSES)
                .frame(art::player::poses::IDLE)
                .swap(art::swaps::SHIRT, shirt)
                .swap(art::swaps::HAIR, hair)
                .swap(art::swaps::SKIN, skin),
            at(-72.0 + i as f32 * 12.0, 50.0, 0.0),
        ));
    }

    // Runners and posers, each holding something by the file's anchors.
    let armed = [
        (art::player::RUN, art::weapons::SHOTGUN, false, palette::clothing::RUST),
        (art::player::JUMP_ARMED, art::weapons::ROCKET_LAUNCHER, false, palette::clothing::GREEN),
        (art::player::CROUCH_WALK, art::weapons::GRENADE_LAUNCHER, true, palette::clothing::PURPLE),
        (art::player::WALL_SLIDE_ARMED, art::weapons::NET_GUN, true, palette::clothing::YELLOW),
        (art::player::IDLE, art::weapons::SWORD, false, palette::clothing::CYAN),
        (art::player::IDLE, art::weapons::FLAG_RED, false, palette::clothing::FLAG_RED),
        (art::player::SLIDE, art::weapons::HANDGUN, false, palette::clothing::PINK),
        (art::player::IDLE, art::weapons::FLAG_BLUE, true, palette::clothing::NAVY),
    ];
    for (i, (pose, weapon, left, shirt)) in armed.into_iter().enumerate() {
        let mut body = commands.spawn((
            sprite(pose).swap(art::swaps::SHIRT, shirt).flipped_x(left),
            at(-84.0 + i as f32 * 24.0, 14.0, 0.0),
        ));
        if a.sprite(pose).is_some_and(|s| s.is_animation()) {
            body.insert(PixelleAnimation::looping());
        }
        let by = body.id();
        commands.spawn((sprite(weapon).flipped_x(left), Held { by }, at(0.0, 0.0, 0.5)));
    }

    // Things that play.
    for (i, (name, y)) in [(art::fx::EXPLOSION, 0.0), (art::weapons::ROCKET, 0.0), (art::weapons::SHELL, 0.0)].into_iter().enumerate() {
        commands.spawn((sprite(name), PixelleAnimation::looping(), at(-90.0 + i as f32 * 40.0, -30.0 + y, 1.0)));
    }
    for (i, e) in [art::emotes::CRY, art::emotes::ANGRY, art::emotes::LAUGH, art::emotes::COOL].into_iter().enumerate() {
        commands.spawn((sprite(e), at(20.0 + i as f32 * 12.0, -30.0, 0.0)));
    }

    // A block of earth from the wall sheet, the way the game's cladding lays one out: the
    // transposed frames are real frames of the baked sheet, and flips make up the rest.
    use art::walls::sheet as w;
    let (cols, rows) = (6, 3);
    for r in 0..rows {
        for c in 0..cols {
            let (edge_x, edge_y) = (c == 0 || c == cols - 1, r == 0 || r == rows - 1);
            let frame = match (edge_x, edge_y) {
                (true, true) => w::CORNER,
                (false, true) => w::TOP,
                (true, false) => w::LEFT,
                (false, false) => w::INTERIOR,
            };
            commands.spawn((
                PixelleSprite {
                    flip_x: c == cols - 1,
                    flip_y: r == rows - 1,
                    ..sprite(art::walls::SHEET).frame(frame)
                },
                at(-100.0 + c as f32 * 8.0, -52.0 - r as f32 * 8.0, -0.9),
            ));
        }
    }
    // A girder, and what stands on the ground.
    use art::platforms::sheet as p;
    for (i, f) in [p::CAP_LEFT, p::MIDDLE, p::MIDDLE, p::MIDDLE, p::CAP_RIGHT].into_iter().enumerate() {
        commands.spawn((sprite(art::platforms::SHEET).frame(f), at(-20.0 + i as f32 * 8.0, -58.0, -0.9)));
    }
    for (i, (name, f)) in [
        (art::props::WEAPON_MARK, 0),
        (art::props::JUMP_PAD, 0),
        (art::props::JUMP_PAD, 1),
        (art::props::SPIKES, 0),
        (art::props::SPIKES, 0),
        (art::props::FLAG_PAD_RED, 0),
        (art::props::FLAG_PAD_BLUE, 0),
    ]
    .into_iter()
    .enumerate()
    {
        commands.spawn((sprite(name).frame(f), at(40.0 + i as f32 * 9.0, -58.0, -0.5)));
    }
    commands.spawn((sprite(art::weapons::BULLET), at(80.0, 14.0, 0.5)));
}

/// Put each held thing in its holder's hand: the holder's `hand` point on the thing's `trigger`,
/// both mirrored when facing left. The hand moves with the pose, so this runs every frame.
fn seat_held(
    arts: Res<Assets<PixelleArt>>,
    holders: Query<(&PixelleSprite, &Transform), Without<Held>>,
    mut held: Query<(&Held, &PixelleSprite, &mut Transform)>,
) {
    for (h, ps, mut t) in &mut held {
        let Ok((holder, ht)) = holders.get(h.by) else { continue };
        let Some(a) = arts.get(&ps.art) else { continue };
        let hand = a.sprite(&holder.name).and_then(|s| s.frame(holder.frame)).and_then(|f| f.offset(art::points::HAND));
        let trigger = a.sprite(&ps.name).and_then(|s| s.frame(ps.frame)).and_then(|f| f.offset(art::points::TRIGGER));
        let (Some(hand), Some(trigger)) = (hand, trigger) else { continue };
        let mut d = hand - trigger;
        if holder.flip_x {
            d.x = -d.x;
        }
        t.translation = ht.translation + d.extend(0.5);
    }
}

fn screenshot(mut commands: Commands, mut frames: Local<u32>, mut exit: MessageWriter<AppExit>) {
    let Ok(path) = std::env::var("PIXELLE_SCREENSHOT") else { return };
    *frames += 1;
    let at: u32 = std::env::var("PIXELLE_SCREENSHOT_FRAME").ok().and_then(|v| v.parse().ok()).unwrap_or(90);
    if *frames == at {
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    }
    if *frames == at + 40 {
        exit.write(AppExit::Success);
    }
}
