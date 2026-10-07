//! Every colour run-2d names, copied from its source with the line it lives on, so the file can be
//! checked against the game by eye. Floats are converted the way the game converts them: `round(v
//! * 255)`.

/// DawnBringer's 32-colour palette, in the order run-2d's indexed sheets store it, with the names
/// it is usually published with.
pub const DB32: [(&str, u32); 32] = [
    ("black", 0x000000),
    ("valhalla", 0x222034),
    ("loulou", 0x45283c),
    ("oiled cedar", 0x663931),
    ("rope", 0x8f563b),
    ("tahiti gold", 0xdf7126),
    ("twine", 0xd9a066),
    ("pancho", 0xeec39a),
    ("golden fizz", 0xfbf236),
    ("atlantis", 0x99e550),
    ("christi", 0x6abe30),
    ("elf green", 0x37946e),
    ("dell", 0x4b692f),
    ("verdigris", 0x524b24),
    ("opal", 0x323c39),
    ("deep koamaru", 0x3f3f74),
    ("venice blue", 0x306082),
    ("royal blue", 0x5b6ee1),
    ("cornflower", 0x639bff),
    ("viking", 0x5fcde4),
    ("light steel blue", 0xcbdbfc),
    ("white", 0xffffff),
    ("heather", 0x9badb7),
    ("topaz", 0x847e87),
    ("dim gray", 0x696a6a),
    ("smokey ash", 0x595652),
    ("clairvoyant", 0x76428a),
    ("brown", 0xac3232),
    ("mandy", 0xd95763),
    ("plum", 0xd77bba),
    ("rain forest", 0x8f974a),
    ("stinger", 0x8a6f30),
];

pub fn hex(v: u32) -> [u8; 4] {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8, 255]
}

pub fn srgb(r: f32, g: f32, b: f32) -> [u8; 4] {
    srgba(r, g, b, 1.0)
}

pub fn srgba(r: f32, g: f32, b: f32, a: f32) -> [u8; 4] {
    let c = |v: f32| (v * 255.0).round() as u8;
    [c(r), c(g), c(b), c(a)]
}

/// `appearance.rs:57-74`: the tones `player.png` is drawn in, which a player picks for themselves.
pub const PLAYER_KEYS: [(&str, u32); 5] = [
    ("hair", 0x222034),
    ("shirt", 0xffffff),
    ("pants", 0x306082),
    ("shoes", 0x8f563b),
    ("skin", 0xeec39a),
];

/// `net.rs:911`: `PALETTE`, one list for every clothing slot. Order is the game's index order.
pub const CLOTHING: [(&str, [f32; 3]); 13] = [
    ("near-black", [0.13, 0.13, 0.20]),
    ("brown", [0.45, 0.30, 0.18]),
    ("blond", [0.85, 0.62, 0.24]),
    ("rust", [0.78, 0.27, 0.20]),
    ("navy", [0.19, 0.38, 0.51]),
    ("green", [0.25, 0.60, 0.42]),
    ("purple", [0.55, 0.30, 0.62]),
    ("pink", [0.90, 0.45, 0.55]),
    ("white", [0.93, 0.93, 0.90]),
    ("grey", [0.55, 0.57, 0.60]),
    ("yellow", [0.95, 0.82, 0.30]),
    ("cyan", [0.35, 0.75, 0.85]),
    ("flag red", [0.675, 0.196, 0.196]),
];

/// `net.rs:948`: `SKIN_TONES`, pale to deep; the first is the sheet's own.
pub const SKIN_TONES: [[f32; 3]; 8] = [
    [0.933, 0.765, 0.604],
    [0.973, 0.863, 0.753],
    [0.878, 0.663, 0.486],
    [0.776, 0.510, 0.353],
    [0.639, 0.373, 0.235],
    [0.486, 0.263, 0.149],
    [0.322, 0.188, 0.110],
    [0.200, 0.125, 0.102],
];

/// `net.rs:429`: `pad_colour`, the dots a map diagram marks weapons with.
pub const WEAPON_MARKERS: [(&str, [f32; 3]); 7] = [
    ("shotgun", [0.88, 0.35, 0.29]),
    ("handgun", [0.94, 0.64, 0.29]),
    ("grenade launcher", [0.48, 0.82, 0.42]),
    ("rocket launcher", [0.36, 0.66, 0.92]),
    ("grenade", [0.42, 0.75, 0.19]),
    ("sword", [0.72, 0.45, 0.85]),
    ("net gun", [0.86, 0.90, 0.94]),
];

/// `menu.rs:38-48`, `killfeed.rs:51`, `ui.rs`, `editor.rs:623`: the interface.
pub const UI: [(&str, [f32; 4]); 14] = [
    ("ink", [0.88, 0.90, 0.94, 1.0]),
    ("dim", [0.55, 0.58, 0.64, 1.0]),
    ("panel", [0.09, 0.10, 0.13, 0.92]),
    ("button", [0.18, 0.20, 0.26, 1.0]),
    ("button hover", [0.26, 0.29, 0.37, 1.0]),
    ("button pressed", [0.34, 0.38, 0.48, 1.0]),
    ("field", [0.13, 0.14, 0.18, 1.0]),
    ("preview", [0.66, 0.68, 0.72, 1.0]),
    ("notice", [0.95, 0.72, 0.40, 1.0]),
    ("killfeed row", [0.09, 0.10, 0.13, 0.82]),
    ("text", [1.0, 1.0, 1.0, 0.95]),
    ("intro band", [0.04, 0.05, 0.07, 0.72]),
    ("curtain", [0.0, 0.0, 0.0, 0.6]),
    ("modal dim", [0.0, 0.0, 0.0, 0.55]),
];

/// `editor.rs`: the level editor's overlays and tool colours (as of e59b281).
pub const EDITOR: [(&str, [f32; 4]); 8] = [
    ("grid line", [1.0, 1.0, 1.0, 0.07]),
    ("problem", [0.95, 0.15, 0.15, 0.40]),
    ("neutral spawn", [1.0, 1.0, 1.0, 0.85]),
    ("trouble", [0.90, 0.35, 0.35, 1.0]),
    ("picked", [0.24, 0.40, 0.66, 1.0]),
    ("picked edge", [0.62, 0.80, 1.0, 1.0]),
    ("pad tool", [0.55, 0.85, 0.45, 1.0]),
    ("erase tool", [0.95, 0.35, 0.35, 1.0]),
];

/// `arena.rs:199-201`: how the map picker draws each material.
pub const MATERIALS: [(&str, [f32; 3]); 3] = [
    ("ground", [0.24, 0.20, 0.16]),
    ("platform", [0.30, 0.33, 0.40]),
    ("wall", [0.20, 0.22, 0.30]),
];

/// `cladding.rs:84`, `cladding.rs:95`, `weapon.rs:480`, `arena.rs:265`, `weapon.rs:821`.
pub const SKY: u32 = 0x4b94ba;
pub const TERRAIN_FILL: u32 = 0x8f563b;
pub const BULLET: [f32; 3] = [0.95, 0.85, 0.2];
pub const SPIKE: [f32; 3] = [0.91, 0.26, 0.24];

/// `appearance.rs:36-48`: the net's mesh and how opaque its strands and holes are.
pub const MESH: u32 = 0xf4f6f8;
pub const STRAND_ALPHA: f32 = 0.85;
pub const HOLE_ALPHA: f32 = 0.3;

/// `level.rs:597`: behind a player-made level in the picker.
pub const CUSTOM_LEVEL: [f32; 3] = [0.600, 0.604, 0.612];
