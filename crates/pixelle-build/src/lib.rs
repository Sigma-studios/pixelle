//! Use a Pixelle project from a game's `build.rs`.
//!
//! ```ignore
//! // build.rs
//! fn main() {
//!     pixelle_build::Art::new("assets/art")
//!         .bake_to("assets/art.pxlb")
//!         .run()
//!         .unwrap();
//! }
//! ```
//!
//! ```ignore
//! // src/main.rs
//! include!(concat!(env!("OUT_DIR"), "/art.rs"));
//! // art::weapons::SHOTGUN, art::player::CROUCH, art::player::idle::rects::HEAD, art::points::MUZZLE, ...
//! ```
//!
//! The constants are generated from the project every build, so renaming a sprite in the editor is
//! a compile error at every place the game used the old name, rather than a missing sprite at run
//! time. The baked file is what a web build ships, where reading a folder of text files would
//! mean many more and larger downloads; see [`pixelle_core::bake`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use pixelle_core::ProjectDir;
use pixelle_core::bake::{self, Baked};

pub struct Art {
    source: PathBuf,
    bake_to: Option<PathBuf>,
    module: String,
    crate_path: String,
}

impl Art {
    /// `source` is the project folder, or its `project.pxl`.
    pub fn new(source: impl AsRef<Path>) -> Self {
        let source = source.as_ref();
        let source = if source.file_name().is_some_and(|n| n == pixelle_core::folder::ROOT) {
            source.parent().unwrap_or(Path::new(".")).to_path_buf()
        } else {
            source.to_path_buf()
        };
        Self {
            source,
            bake_to: None,
            module: "art".into(),
            crate_path: "::bevy_pixelle".into(),
        }
    }

    /// Also write the baked runtime file here.
    pub fn bake_to(mut self, path: impl AsRef<Path>) -> Self {
        self.bake_to = Some(path.as_ref().to_path_buf());
        self
    }

    /// The generated module's name, and the file in `OUT_DIR` (`<name>.rs`). Default `art`.
    pub fn module(mut self, name: &str) -> Self {
        self.module = name.into();
        self
    }

    /// Where `SpriteId` lives. Default `::bevy_pixelle`.
    pub fn crate_path(mut self, path: &str) -> Self {
        self.crate_path = path.into();
        self
    }

    /// Bake, generate, and tell Cargo to rerun when the project changes. Returns the generated
    /// file's path.
    pub fn run(self) -> Result<PathBuf, String> {
        // A folder: Cargo looks for changes anywhere inside it.
        println!("cargo:rerun-if-changed={}", self.source.display());
        let project = ProjectDir::load(&self.source).map_err(|e| format!("{}/{e}", self.source.display()))?;
        for problem in project.validate() {
            println!("cargo:warning=pixelle: {problem}");
        }
        let baked = bake::bake_folder(&project);
        if let Some(to) = &self.bake_to {
            let out = baked.to_bytes();
            // Leave the file alone when nothing changed, so asset watchers stay quiet.
            if std::fs::read(to).ok().as_deref() != Some(&out[..]) {
                pixelle_core::folder::write_atomic(to, &out).map_err(|e| format!("{}: {e}", to.display()))?;
            }
        }
        let out_dir = std::env::var_os("OUT_DIR").ok_or("OUT_DIR is not set: call this from build.rs")?;
        let path = Path::new(&out_dir).join(format!("{}.rs", self.module));
        let code = codegen(&baked, &self.module, &self.crate_path, &self.source.display().to_string());
        std::fs::write(&path, code).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    }
}

/// The generated module for a baked project.
pub fn codegen(baked: &Baked, module: &str, crate_path: &str, source: &str) -> String {
    // A tree of modules from the slash-separated sprite names.
    #[derive(Default)]
    struct Node {
        consts: Vec<String>,
        children: BTreeMap<String, Node>,
    }
    let mut root = Node::default();
    let mut points = BTreeSet::new();
    let mut rects = BTreeSet::new();
    for s in &baked.sprites {
        // An animation of a sheet goes beside its frame names: `art::player::anims::RUN`.
        let anim = s.tags.get(pixelle_core::folder::ANIM_OF).and_then(|sheet| {
            let leaf = s.name.rsplit_once(pixelle_core::folder::ANIM_SEPARATOR)?.1;
            Some((sheet.clone(), leaf.to_string()))
        });
        let (dirs, leaf): (Vec<String>, String) = match &anim {
            Some((sheet, leaf)) => (sheet.split('/').map(str::to_string).chain(["anims".to_string()]).collect(), leaf.clone()),
            None => {
                let mut parts: Vec<String> = s.name.split('/').map(str::to_string).collect();
                let leaf = parts.pop().expect("names are not empty");
                (parts, leaf)
            }
        };
        let mut node = &mut root;
        for d in &dirs {
            node = node.children.entry(module_ident(d)).or_default();
        }
        let kind = if s.is_animation() {
            let total: f32 = s.frames.iter().map(|f| f.duration).sum();
            format!("animation, {} frames, {total:.3}s", s.frames.len())
        } else if s.frames.len() > 1 {
            format!("{} frames", s.frames.len())
        } else {
            let r = s.frames[0].rect;
            format!("{}x{}", r.w, r.h)
        };
        node.consts.push(format!(
            "/// `{}`: {kind}.\npub const {}: {crate_path}::SpriteId = {crate_path}::SpriteId({:?});",
            s.name,
            const_ident(&leaf),
            s.name
        ));
        // A `frames` tag names the frames: their indices, in a module of the sprite's name.
        if let Some(names) = s.tags.get("frames") {
            let m = node.children.entry(module_ident(&leaf)).or_default();
            for (i, n) in names.split(',').map(str::trim).enumerate().filter(|(_, n)| !n.is_empty()) {
                m.consts.push(format!("/// Frame {i} of `{}`.\npub const {}: usize = {i};", s.name, const_ident(n)));
                // Its rects and points, as values, for code that needs them before any asset has
                // loaded or with none at all: `art::player::idle::rects::HEAD`.
                let Some(f) = s.frames.get(i) else { continue };
                if f.rects.is_empty() && f.points.is_empty() {
                    continue;
                }
                let fm = m.children.entry(module_ident(n)).or_default();
                if !f.rects.is_empty() {
                    let rm = fm.children.entry("rects".into()).or_default();
                    for (rn, r) in &f.rects {
                        rm.consts.push(format!(
                            "/// `{rn}` on frame {i} of `{}`, from the frame's top-left.\npub const {}: {crate_path}::pixelle_core::IRect = {crate_path}::pixelle_core::IRect::new({}, {}, {}, {});",
                            s.name,
                            const_ident(rn),
                            r.x,
                            r.y,
                            r.w,
                            r.h
                        ));
                    }
                }
                if !f.points.is_empty() {
                    let pm = fm.children.entry("points".into()).or_default();
                    for (pn, p) in &f.points {
                        pm.consts.push(format!(
                            "/// `{pn}` on frame {i} of `{}`, from the frame's top-left.\npub const {}: {crate_path}::pixelle_core::IVec2 = {crate_path}::pixelle_core::IVec2::new({}, {});",
                            s.name,
                            const_ident(pn),
                            p.x,
                            p.y
                        ));
                    }
                }
            }
        }
        for f in &s.frames {
            points.extend(f.points.keys().cloned());
            rects.extend(f.rects.keys().cloned());
        }
    }

    let mut out = String::new();
    let _ = writeln!(out, "// Generated by pixelle-build from {source}. Do not edit.\n");
    let _ = writeln!(out, "#[allow(dead_code, clippy::all)]\npub mod {module} {{");
    fn write_node(out: &mut String, node: &Node, depth: usize) {
        let pad = "    ".repeat(depth);
        for c in &node.consts {
            for line in c.lines() {
                let _ = writeln!(out, "{pad}{line}");
            }
        }
        for (name, child) in &node.children {
            let _ = writeln!(out, "{pad}pub mod {name} {{");
            write_node(out, child, depth + 1);
            let _ = writeln!(out, "{pad}}}");
        }
    }
    write_node(&mut out, &root, 1);

    let names = |out: &mut String, title: &str, doc: &str, items: &mut dyn Iterator<Item = String>| {
        let _ = writeln!(out, "    /// {doc}\n    pub mod {title} {{");
        for n in items {
            let _ = writeln!(out, "        pub const {}: &str = {n:?};", const_ident(&n));
        }
        let _ = writeln!(out, "    }}");
    };
    names(&mut out, "points", "Anchor point names used by any frame.", &mut points.into_iter());
    let _ = writeln!(out, "    /// Terrains, for autotiling grids at run time.\n    pub mod terrains {{");
    for t in &baked.terrains {
        let _ = writeln!(
            out,
            "        pub const {}: {crate_path}::TerrainId = {crate_path}::TerrainId({:?});",
            const_ident(&t.name),
            t.name
        );
    }
    let _ = writeln!(out, "    }}");
    names(&mut out, "rects", "Named rectangle names used by any frame.", &mut rects.into_iter());
    names(
        &mut out,
        "swaps",
        "Recolour slots.",
        &mut baked.swaps.iter().map(|s| s.name.clone()),
    );

    let _ = writeln!(out, "    /// Palette indices by section and name.\n    pub mod palette {{");
    for sec in &baked.sections {
        let _ = writeln!(out, "        pub mod {} {{", module_ident(&sec.label));
        let mut seen = BTreeSet::new();
        for i in sec.start..sec.start + sec.len {
            let mut id = const_ident(&baked.palette_names[i as usize]);
            while !seen.insert(id.clone()) {
                id.push('_');
            }
            let _ = writeln!(out, "            pub const {id}: u8 = {i};");
        }
        let _ = writeln!(out, "        }}");
    }
    let _ = writeln!(out, "    }}\n}}");
    out
}

fn words(s: &str) -> String {
    let snake = pixelle_core::import::snake(s);
    let mut out: String = snake
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if out.is_empty() {
        out.push('_');
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

fn const_ident(s: &str) -> String {
    words(s).to_uppercase()
}

fn module_ident(s: &str) -> String {
    let w = words(s);
    const KEYWORDS: &[&str] = &[
        "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
        "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
        "static", "struct", "trait", "true", "type", "unsafe", "use", "where", "while", "async",
        "await", "dyn", "abstract", "become", "box", "do", "final", "macro", "override", "priv",
        "typeof", "unsized", "virtual", "yield", "try", "gen",
    ];
    match w.as_str() {
        "self" | "super" | "crate" | "Self" => format!("{w}_"),
        _ if KEYWORDS.contains(&w.as_str()) => format!("r#{w}"),
        _ => w,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_valid_rust() {
        assert_eq!(const_ident("rocket launcher"), "ROCKET_LAUNCHER");
        assert_eq!(const_ident("#ff0000"), "FF0000");
        assert_eq!(const_ident("2nd"), "_2ND");
        assert_eq!(module_ident("DawnBringer 32"), "dawn_bringer_32");
        assert_eq!(module_ident("type"), "r#type");
    }

    #[test]
    fn a_named_frames_rects_and_points_are_values() {
        let mut p = ProjectDir::new("t");
        let mut s = pixelle_core::SpriteFile::new(8, 8);
        let f = &mut s.frames[0];
        f.name = Some("idle".into());
        f.rects.insert("head".into(), pixelle_core::IRect::new(3, 0, 3, 2));
        f.points.insert("hand".into(), pixelle_core::IVec2::new(5, 4));
        p.sprites.insert("player".into(), s);
        let code = codegen(&bake::bake_folder(&p), "art", "::x", "test");
        assert!(code.contains("pub mod idle {"), "{code}");
        assert!(code.contains("pub const HEAD: ::x::pixelle_core::IRect = ::x::pixelle_core::IRect::new(3, 0, 3, 2);"), "{code}");
        assert!(code.contains("pub const HAND: ::x::pixelle_core::IVec2 = ::x::pixelle_core::IVec2::new(5, 4);"), "{code}");
    }
}
