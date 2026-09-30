//! `pixelle`: look inside, render and pack `.pxl` projects without opening the editor.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use pixelle_core::layer::LayerKind;
use pixelle_core::render::Renderer;
use pixelle_core::{IRect, Project, format, pack, png_io};

#[derive(Parser)]
#[command(name = "pixelle", about = "Inspect, render and pack Pixelle projects")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Summarise a project: palette, layers, sprites, terrains. Exits non-zero if it has problems.
    Info {
        file: PathBuf,
        /// Dump the whole manifest as RON instead (pixel data summarised).
        #[arg(long)]
        ron: bool,
    },
    /// Render the canvas (every visible layer) to a PNG.
    Render {
        file: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        /// Whole-number upscale.
        #[arg(long, default_value_t = 1)]
        scale: u32,
        /// Only this part of the canvas: x,y,w,h. Defaults to everything, or a bookmark's name.
        #[arg(long)]
        rect: Option<String>,
        /// Background colour as RRGGBB or RRGGBBAA; transparent by default.
        #[arg(long)]
        bg: Option<String>,
    },
    /// Pack every sprite into one atlas PNG, with a RON file of where each frame landed.
    Pack {
        file: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, default_value_t = 1)]
        gutter: u32,
        /// Write an 8-bit palette PNG (the project palette) instead of RGBA.
        #[arg(long)]
        indexed: bool,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Info { file, ron } => info(&load(&file)?, ron),
        Command::Render {
            file,
            out,
            scale,
            rect,
            bg,
        } => {
            let project = load(&file)?;
            let rect = match rect {
                None => project.bounds(),
                Some(r) => parse_rect(&project, &r)?,
            };
            if rect.is_empty() {
                bail!("nothing to render");
            }
            let bg = bg.as_deref().map(parse_hex).transpose()?.unwrap_or([0; 4]);
            let img = Renderer::new(&project).rgba(rect, bg).scaled(scale);
            std::fs::write(&out, png_io::encode_rgba(&img)?)?;
            println!("{} ({}x{}, canvas {:?})", out.display(), img.w, img.h, rect);
            Ok(())
        }
        Command::Pack {
            file,
            out,
            gutter,
            indexed,
        } => {
            let project = load(&file)?;
            let atlas = pack::pack(&project, gutter);
            let img = atlas.image();
            let bytes = if indexed {
                png_io::encode_indexed(&img, &project.palette)?
            } else {
                png_io::encode_rgba(&img.to_rgba(&project.palette))?
            };
            std::fs::write(&out, bytes)?;
            let map: BTreeMap<&String, Vec<[i32; 4]>> = atlas
                .sprites
                .iter()
                .map(|(k, v)| (k, v.iter().map(|r| [r.x, r.y, r.w as i32, r.h as i32]).collect()))
                .collect();
            let ron_path = out.with_extension("ron");
            std::fs::write(
                &ron_path,
                ron::ser::to_string_pretty(&map, ron::ser::PrettyConfig::default())?,
            )?;
            println!(
                "{} ({}x{}, {} sprites) + {}",
                out.display(),
                atlas.width,
                atlas.height,
                atlas.sprites.len(),
                ron_path.display()
            );
            Ok(())
        }
    }
}

fn load(path: &PathBuf) -> Result<Project> {
    format::load_file(path).with_context(|| format!("reading {}", path.display()))
}

fn parse_rect(project: &Project, s: &str) -> Result<IRect> {
    if let Some(b) = project.bookmarks.iter().find(|b| b.name == s) {
        return Ok(b.rect);
    }
    let n: Vec<i64> = s
        .split(',')
        .map(|v| v.trim().parse())
        .collect::<Result<_, _>>()
        .context("--rect is x,y,w,h or a bookmark name")?;
    let [x, y, w, h] = n[..] else {
        bail!("--rect is x,y,w,h or a bookmark name");
    };
    Ok(IRect::new(x as i32, y as i32, w as u32, h as u32))
}

fn parse_hex(s: &str) -> Result<[u8; 4]> {
    let s = s.trim_start_matches('#');
    let byte = |i: usize| u8::from_str_radix(&s[i..i + 2], 16);
    Ok(match s.len() {
        6 => [byte(0)?, byte(2)?, byte(4)?, 255],
        8 => [byte(0)?, byte(2)?, byte(4)?, byte(6)?],
        _ => bail!("colours are RRGGBB or RRGGBBAA"),
    })
}

fn hex(c: [u8; 4]) -> String {
    if c[3] == 255 {
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3])
    }
}

fn info(p: &Project, as_ron: bool) -> Result<()> {
    if as_ron {
        println!(
            "{}",
            ron::ser::to_string_pretty(p, ron::ser::PrettyConfig::default().struct_names(false))?
        );
        return Ok(());
    }
    let b = p.bounds();
    println!("{} — canvas {}x{} at ({}, {}), grid {}", p.name, b.w, b.h, b.x, b.y, p.grid);

    println!("\npalette: {} entries", p.palette.len());
    for s in p.palette.sections() {
        let names: Vec<String> = s
            .indices()
            .map(|i| {
                let e = p.palette.entry(i).unwrap();
                format!("{} {}", e.name, hex(e.rgba))
            })
            .collect();
        println!("  [{:>3}..{:>3}) {:<16} {}", s.start, s.start + s.len, s.label, names.join(", "));
    }

    println!("\nlayers (bottom to top):");
    for l in &p.layers {
        let detail = match &l.kind {
            LayerKind::Pixels(g) => format!("{} chunks", g.chunk_count()),
            LayerKind::Terrain(t) => format!("{} cells of {}px", t.cells.iter().count(), t.cell_size),
            LayerKind::Instances(v) => format!("{} instances", v.len()),
        };
        let flags = [(l.reference, " reference"), (!l.visible, " hidden"), (l.locked, " locked")]
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, s)| *s)
            .collect::<String>();
        println!("  #{:<2} {:<12} {:<9} {}{}", l.id, l.name, l.kind_name(), detail, flags);
    }

    println!("\nsprites: {}", p.sprites.len());
    for s in &p.sprites {
        let layers: Vec<&str> = s
            .layers
            .iter()
            .filter_map(|id| p.layer(*id).map(|l| l.name.as_str()))
            .collect();
        let f0 = &s.frames[0];
        let (w, h) = f0.size();
        let anim = if s.is_animation() {
            let total: f32 = s.frames.iter().map(|f| f.duration).sum();
            format!(" anim {:.3}s", total)
        } else {
            String::new()
        };
        let mut extra: Vec<String> = f0.points.iter().map(|(k, v)| format!("{k}@{},{}", v.x, v.y)).collect();
        extra.extend(f0.rects.iter().map(|(k, r)| format!("{k}[{},{} {}x{}]", r.x, r.y, r.w, r.h)));
        extra.extend(s.frames.iter().enumerate().filter(|(_, f)| !f.transform.is_identity()).map(|(i, f)| {
            format!("f{i}:{}", f.transform)
        }));
        println!(
            "  {:<24} {}x{} x{}{} [{}] {}",
            s.name,
            w,
            h,
            s.frames.len(),
            anim,
            layers.join("+"),
            extra.join(" ")
        );
    }

    if !p.tilesets.is_empty() {
        println!("\ntilesets:");
        for t in &p.tilesets {
            println!("  {:<12} {} tiles of {}px at ({}, {})", t.name, t.count, t.tile_size, t.origin.x, t.origin.y);
        }
    }
    if !p.terrains.is_empty() {
        println!("\nterrains:");
        for (i, t) in p.terrains.iter().enumerate() {
            let rules: Vec<&str> = t.rules.iter().map(|r| r.name.as_str()).collect();
            println!("  {} {:<12} on {:<10} rules: {}", i + 1, t.name, t.tileset, rules.join(", "));
        }
    }
    if !p.swaps.is_empty() {
        println!("\nswap slots:");
        for s in &p.swaps {
            let e = p.palette.entry(s.index).unwrap();
            println!("  {:<8} index {:>3} ({}) from {:?}", s.name, s.index, hex(e.rgba), s.options);
        }
    }
    if !p.bookmarks.is_empty() {
        println!("\nbookmarks:");
        for b in &p.bookmarks {
            println!("  {:<10} {},{} {}x{}", b.name, b.rect.x, b.rect.y, b.rect.w, b.rect.h);
        }
    }

    let problems = p.validate();
    if problems.is_empty() {
        println!("\nno problems");
        Ok(())
    } else {
        println!("\nproblems:");
        for problem in &problems {
            println!("  - {problem}");
        }
        bail!("{} problem(s)", problems.len())
    }
}
