//! `pixelle`: look inside, bake and import into project folders without opening the editor.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use pixelle_core::folder::{self, ProjectDir};
use pixelle_core::palette::presets;
use pixelle_core::{bake, import, png_io};

#[derive(Parser)]
#[command(name = "pixelle", about = "Inspect, bake and import into Pixelle projects")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Summarise a project: palette, sprites, terrains. Exits non-zero if it has problems.
    Info { project: PathBuf },
    /// Start a project folder with a preset palette.
    New {
        project: PathBuf,
        /// db32, pico8 or endesga32.
        #[arg(long, default_value = "db32")]
        palette: String,
    },
    /// Write the baked file a web build ships: packed atlas, names, anchors, recolour slots.
    Bake {
        project: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Write the packed atlas as a PNG.
    Atlas {
        project: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Write one sprite's frames side by side as a PNG.
    Sheet {
        project: PathBuf,
        sprite: String,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Print (or write) the typed constants `pixelle-build` generates.
    Codegen {
        project: PathBuf,
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[arg(long, default_value = "art")]
        module: String,
    },
    /// Make a sprite from a PNG sheet, matching colours exactly and adding what is new.
    ImportPng {
        project: PathBuf,
        png: PathBuf,
        /// The sprite's name; defaults to the file's name.
        #[arg(long)]
        name: Option<String>,
        /// Cut the sheet into frames this wide; the whole picture by default.
        #[arg(long, default_value_t = 0)]
        frame_width: u32,
    },
    /// Make sprites from an .aseprite file: one per tag, with its timings.
    ImportAseprite {
        project: PathBuf,
        aseprite: PathBuf,
        /// Sprite name (prefix, when the file has tags). Defaults to the file's name.
        #[arg(long)]
        name: Option<String>,
        /// Keep each Aseprite layer a layer of its own, hidden ones included but not exported.
        #[arg(long)]
        split_layers: bool,
    },
    /// Add a palette (.hex or .gpl) as a new section.
    ImportPalette {
        project: PathBuf,
        palette: PathBuf,
        /// Section label; defaults to the file's name.
        #[arg(long)]
        section: Option<String>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Info { project } => info(&load(&project)?),
        Command::New { project, palette } => {
            if project.join(folder::ROOT).exists() {
                bail!("{} is already a project", project.display());
            }
            let name = stem(&project);
            let mut p = ProjectDir::new(name);
            let preset = presets::ALL
                .into_iter()
                .find(|(n, _)| n.to_lowercase().replace([' ', '-'], "").starts_with(&palette.to_lowercase()))
                .with_context(|| format!("no palette {palette:?}; try db32, pico8 or endesga32"))?;
            presets::add(&mut p.palette, preset)?;
            save(&p, &project)
        }
        Command::Bake { project, out } => {
            let baked = bake::bake_folder(&load(&project)?);
            std::fs::write(&out, baked.to_bytes())?;
            println!("{}: {} sprites, {}x{} atlas", out.display(), baked.sprites.len(), baked.atlas_width, baked.atlas_height);
            Ok(())
        }
        Command::Atlas { project, out } => {
            let baked = bake::bake_folder(&load(&project)?);
            let img = pixelle_core::render::RgbaImage { w: baked.atlas_width, h: baked.atlas_height, px: baked.atlas_rgba() };
            std::fs::write(&out, png_io::encode_rgba(&img)?)?;
            Ok(())
        }
        Command::Sheet { project, sprite, out } => {
            let baked = bake::bake_folder(&load(&project)?);
            let img = baked.sheet_rgba(&sprite).with_context(|| format!("no sprite {sprite:?}"))?;
            std::fs::write(&out, png_io::encode_rgba(&img)?)?;
            Ok(())
        }
        Command::Codegen { project, out, module } => {
            let baked = bake::bake_folder(&load(&project)?);
            let code = pixelle_build::codegen(&baked, &module, "::bevy_pixelle", &project.display().to_string());
            match out {
                Some(out) => std::fs::write(out, code)?,
                None => print!("{code}"),
            }
            Ok(())
        }
        Command::ImportPng { project, png, name, frame_width } => {
            let mut p = load(&project)?;
            let img = png_io::decode(&std::fs::read(&png)?)?.to_rgba();
            let mut r = import::Report::default();
            let sprite = import::sheet(&mut p.palette, &img, frame_width, None, &mut r).map_err(anyhow::Error::msg)?;
            let name = name.unwrap_or_else(|| stem(&png));
            println!("{name}: {} frames", sprite.frames.len());
            p.sprites.insert(name, sprite);
            report(&r);
            save(&p, &project)
        }
        Command::ImportAseprite { project, aseprite, name, split_layers } => {
            let mut p = load(&project)?;
            let ase = pixelle_core::aseprite::parse(&std::fs::read(&aseprite)?)?;
            let name = name.unwrap_or_else(|| stem(&aseprite));
            let mut r = import::Report::default();
            for (n, s) in import::aseprite(&mut p.palette, &ase, &name, split_layers, &mut r).map_err(anyhow::Error::msg)? {
                println!("{n}: {} frames", s.frames.len());
                p.sprites.insert(n, s);
            }
            report(&r);
            save(&p, &project)
        }
        Command::ImportPalette { project, palette, section } => {
            let mut p = load(&project)?;
            let text = std::fs::read_to_string(&palette)?;
            let colours = match palette.extension().and_then(|e| e.to_str()) {
                Some("gpl") => import::parse_gpl(&text),
                _ => import::parse_hex(&text),
            }
            .map_err(anyhow::Error::msg)?;
            p.palette.begin_section(section.unwrap_or_else(|| stem(&palette)));
            for (n, c) in &colours {
                p.palette.push(n, *c)?;
            }
            println!("added {} colours", colours.len());
            save(&p, &project)
        }
    }
}

fn stem(path: &Path) -> String {
    path.file_stem().map_or("untitled".into(), |s| s.to_string_lossy().into_owned())
}

/// A project from its folder, or from its `project.pxl`.
fn load(path: &Path) -> Result<ProjectDir> {
    Ok(ProjectDir::load(folder_of(path))?)
}

fn folder_of(path: &Path) -> &Path {
    if path.file_name().is_some_and(|n| n == folder::ROOT) { path.parent().unwrap_or(Path::new(".")) } else { path }
}

fn save(p: &ProjectDir, path: &Path) -> Result<()> {
    for f in p.save(folder_of(path))? {
        println!("wrote {f}");
    }
    Ok(())
}

fn report(r: &import::Report) {
    if !r.added.is_empty() {
        println!("added {} colours to the palette", r.added.len());
    }
}

fn info(p: &ProjectDir) -> Result<()> {
    println!("{}: {} colours in {} sections", p.name, p.palette.len() - 1, p.palette.sections().len());
    for s in p.palette.sections() {
        println!("  {} ({})", s.label, s.len);
    }
    for (name, s) in &p.sprites {
        let total: f32 = s.frames.iter().map(|f| f.duration).sum();
        let kind = match s.frames.len() {
            1 => format!("{}x{}", s.frames[0].w, s.frames[0].h),
            n if total > 0.0 => format!("animation, {n} frames, {total:.3}s"),
            n => format!("{n} frames"),
        };
        println!("sprite {name}: {kind}");
    }
    for (name, t) in &p.terrains {
        println!("terrain {name}: {} ({} tiles of {}px)", t.preset.label(), t.tiles.len(), t.cell);
    }
    let problems = p.validate();
    for problem in &problems {
        eprintln!("problem: {problem}");
    }
    if !problems.is_empty() {
        bail!("{} problems", problems.len());
    }
    Ok(())
}
