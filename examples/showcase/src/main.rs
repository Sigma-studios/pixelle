//! run-2d's art, straight from the project folder `assets/run2d`.
//!
//! Everything here is looked up by name through the generated `art` module — sprites, a sheet's
//! frames and its animations by their names, no pixel constants. Weapons sit in hands by the project's own
//! `hand` and `trigger` points; players are recoloured through its recolour slots; a grid map is
//! autotiled by its walls terrain.
//!
//! `cargo run -p showcase` loads the folder; edit any file in it in the editor and it reloads.
//! `PIXELLE_FILE=run2d.pxlb` loads the baked file a web build would; `PIXELLE_SCREENSHOT=out.png`
//! saves a screenshot and quits.

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
        .unwrap_or_else(|_| if cfg!(target_arch = "wasm32") { "run2d.pxlb" } else { "run2d/project.pxl" }.into());
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
            sprite(art::PLAYER)
                .frame(art::player::IDLE)
                .swap(art::swaps::SHIRT, shirt)
                .swap(art::swaps::HAIR, hair)
                .swap(art::swaps::SKIN, skin),
            at(-72.0 + i as f32 * 12.0, 50.0, 0.0),
        ));
    }

    // Runners and posers, each holding something by the project's anchors. The cycles are
    // animations of the sheet, played by name; the poses are frames of it.
    use art::{player as pose, weapons as weapon};
    enum Body {
        Plays(bevy_pixelle::SpriteId),
        Pose(usize),
    }
    let armed = [
        (Body::Plays(pose::anims::RUN), weapon::SHOTGUN, false, palette::clothing::RUST),
        (Body::Pose(pose::JUMP_ARMED), weapon::ROCKET_LAUNCHER, false, palette::clothing::GREEN),
        (Body::Plays(pose::anims::CROUCH_WALK), weapon::GRENADE_LAUNCHER, true, palette::clothing::PURPLE),
        (Body::Pose(pose::WALL_SLIDE_ARMED), weapon::NET_GUN, true, palette::clothing::YELLOW),
        (Body::Pose(pose::IDLE), weapon::SWORD, false, palette::clothing::CYAN),
        (Body::Pose(pose::IDLE), weapon::FLAG_RED, false, palette::clothing::FLAG_RED),
        (Body::Pose(pose::SLIDE), weapon::HANDGUN, false, palette::clothing::PINK),
        (Body::Pose(pose::IDLE), weapon::FLAG_BLUE, true, palette::clothing::NAVY),
    ];
    for (i, (body, held, left, shirt)) in armed.into_iter().enumerate() {
        let (look, plays) = match body {
            Body::Plays(anim) => (sprite(anim), true),
            Body::Pose(frame) => (sprite(art::PLAYER).frame(frame), false),
        };
        let mut e = commands.spawn((look.swap(art::swaps::SHIRT, shirt).flipped_x(left), at(-84.0 + i as f32 * 24.0, 14.0, 0.0)));
        if plays {
            e.insert(PixelleAnimation::looping());
        }
        let by = e.id();
        commands.spawn((sprite(art::WEAPONS).frame(held).flipped_x(left), Held { by }, at(0.0, 0.0, 0.5)));
    }

    // Things that play: animations of their own, and animations of a sheet.
    for (anim, x, y) in [(art::fx::EXPLOSION, 104.0, -22.0), (weapon::anims::ROCKET, -30.0, -18.0), (weapon::anims::SHELL, -10.0, -18.0)] {
        commands.spawn((sprite(anim), PixelleAnimation::looping(), at(x, y, 1.0)));
    }
    for i in 0..a.sprite(art::EMOTES).map_or(0, |s| s.len()) {
        commands.spawn((sprite(art::EMOTES).frame(i), at(20.0 + i as f32 * 12.0, -18.0, 0.0)));
    }

    // A grid map, clad at run time by the file's walls terrain — whole frames where they fit,
    // quarters elsewhere — the way a grid-painted run-2d level is. Row 0 is the bottom.
    const MAP: [&str; 5] = [
        "##.....#.",
        "#......#.",
        "#..##..##",
        "#..##...#",
        "#########",
    ];
    let walls = a.terrain(art::terrains::WALLS).expect("the walls terrain");
    let size = walls.cell_size() as f32;
    let ground = |c: IVec2| {
        let row = MAP.len() as i32 - 1 - c.y;
        (0..MAP.len() as i32).contains(&row)
            && MAP[row as usize].as_bytes().get(c.x as usize).is_some_and(|b| *b == b'#')
    };
    for row in 0..MAP.len() as i32 {
        for col in 0..MAP[0].len() as i32 {
            let cell = IVec2::new(col, row);
            if !ground(cell) {
                continue;
            }
            let centre = Vec2::new(-116.0 + col as f32 * size, -68.0 + row as f32 * size);
            for piece in walls.cell(|d| ground(cell + d)) {
                commands.spawn((walls.sprite(&piece), Transform::from_translation((centre + piece.offset).extend(-0.9))));
            }
        }
    }
    // A girder, and what stands on the ground.
    use art::{platforms as p, props};
    for (i, f) in [p::CAP_LEFT, p::MIDDLE, p::MIDDLE, p::MIDDLE, p::CAP_RIGHT].into_iter().enumerate() {
        commands.spawn((sprite(art::PLATFORMS).frame(f), at(-20.0 + i as f32 * 8.0, -58.0, -0.9)));
    }
    for (i, f) in [props::WEAPON_MARK, props::JUMP_PAD_REST, props::JUMP_PAD_SPRUNG, props::SPIKES, props::SPIKES, props::PORTAL_SINGLE]
        .into_iter()
        .enumerate()
    {
        commands.spawn((sprite(art::PROPS).frame(f), at(40.0 + i as f32 * 9.0, -58.0, -0.5)));
    }
    commands.spawn((sprite(art::fx::BULLET), at(80.0, 14.0, 0.5)));
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
