//! Bringing outside art and palettes into a project.

use crate::aseprite::Ase;
use crate::geom::{IRect, IVec2};
use crate::layer::{LayerId, LayerKind};
use crate::palette::{PaletteFull, Rgba};
use crate::project::Project;
use crate::render::RgbaImage;
use crate::sprite::{Frame, Sprite};

/// Colours of a Lospec-style `.hex` file: one `RRGGBB` per line.
pub fn parse_hex(text: &str) -> Result<Vec<(String, Rgba)>, String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with(';') && !l.starts_with("//"))
        .enumerate()
        .map(|(i, l)| {
            let l = l.trim_start_matches('#');
            let v = u32::from_str_radix(l, 16).map_err(|_| format!("line {}: {l:?} is not a colour", i + 1))?;
            Ok((format!("#{l}"), [(v >> 16) as u8, (v >> 8) as u8, v as u8, 255]))
        })
        .collect()
}

/// Colours of a GIMP `.gpl` palette, with their names.
pub fn parse_gpl(text: &str) -> Result<Vec<(String, Rgba)>, String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with("GIMP")
            || line.starts_with("Name:")
            || line.starts_with("Columns:")
        {
            continue;
        }
        let mut parts = line.split_whitespace();
        let mut channel = || -> Result<u8, String> {
            parts
                .next()
                .and_then(|p| p.parse().ok())
                .ok_or_else(|| format!("line {}: expected three numbers", n + 1))
        };
        let rgb = [channel()?, channel()?, channel()?];
        let name: Vec<&str> = parts.collect();
        let name = if name.is_empty() {
            format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
        } else {
            name.join(" ")
        };
        out.push((name, [rgb[0], rgb[1], rgb[2], 255]));
    }
    Ok(out)
}

/// Where imported colours no section names go: an "Imported" section at the end of the palette,
/// or a new one if the existing one is not last and cannot grow.
fn imported_section(project: &mut Project) -> String {
    let pal = &project.palette;
    let last = pal.sections().last();
    if let Some(s) = last
        && s.label.starts_with("Imported")
        && (s.start + s.len) as usize == pal.len()
    {
        return s.label.clone();
    }
    let n = pal.sections().iter().filter(|s| s.label.starts_with("Imported")).count();
    let label = if n == 0 { "Imported".to_string() } else { format!("Imported {}", n + 1) };
    project.palette.begin_section(label.clone());
    label
}

/// What an import did to the palette.
#[derive(Debug, Default)]
pub struct Report {
    /// Colours that matched nothing and were added.
    pub added: Vec<Rgba>,
}

/// The index for a colour: an exact match anywhere (preferring `prefer`), or a new entry.
fn index_for(project: &mut Project, rgba: Rgba, prefer: Option<&str>, report: &mut Report) -> Result<u8, PaletteFull> {
    if rgba[3] == 0 {
        return Ok(0);
    }
    if let Some(i) = prefer.and_then(|s| project.palette.find(rgba, Some(s))) {
        return Ok(i);
    }
    if let Some(i) = project.palette.find(rgba, None) {
        return Ok(i);
    }
    let _ = imported_section(project);
    report.added.push(rgba);
    project
        .palette
        .push(format!("#{:02x}{:02x}{:02x}", rgba[0], rgba[1], rgba[2]), rgba)
}

/// Paint an RGBA picture onto a pixel layer, matching colours to the palette exactly and adding
/// what does not match.
pub fn paint_rgba(
    project: &mut Project,
    layer: LayerId,
    at: IVec2,
    img: &RgbaImage,
    prefer: Option<&str>,
    report: &mut Report,
) -> Result<(), String> {
    let mut indices = Vec::with_capacity((img.w * img.h) as usize);
    for c in img.px.chunks_exact(4) {
        indices.push(index_for(project, [c[0], c[1], c[2], c[3]], prefer, report).map_err(|e| e.to_string())?);
    }
    let grid = project
        .layer_mut(layer)
        .and_then(|l| l.pixels_mut())
        .ok_or("not a pixel layer")?;
    for y in 0..img.h {
        for x in 0..img.w {
            let v = indices[(y * img.w + x) as usize];
            if v != 0 {
                grid.set(at + IVec2::new(x as i32, y as i32), v);
            }
        }
    }
    Ok(())
}

/// Import an Aseprite file: frames laid out left to right from `at`, visible layers flattened onto
/// `layer` (or, with `split`, each layer onto a new pixel layer of its name), and a sprite per tag
/// — or one for the whole file if it has none — named under `name`.
pub fn aseprite(
    project: &mut Project,
    ase: &Ase,
    at: IVec2,
    layer: LayerId,
    name: &str,
    split: bool,
    report: &mut Report,
) -> Result<Vec<String>, String> {
    let (w, h) = (ase.width, ase.height);
    let mut targets = Vec::new();
    for (l, al) in ase.layers.iter().enumerate() {
        if al.group || (!split && !al.visible) {
            continue;
        }
        let target = if split {
            let id = project.add_layer(format!("{name}/{}", al.name), LayerKind::Pixels(Default::default()));
            project.layer_mut(id).unwrap().visible = al.visible;
            id
        } else {
            layer
        };
        targets.push((l, target));
    }
    for f in 0..ase.frames.len() {
        let origin = at + IVec2::new(f as i32 * w as i32, 0);
        if split {
            for &(l, target) in &targets {
                let img = to_image(&ase.frames[f][l], w, h);
                paint_rgba(project, target, origin, &img, None, report)?;
            }
        } else {
            let img = to_image(&ase.visible(f), w, h);
            paint_rgba(project, layer, origin, &img, None, report)?;
        }
    }

    let layers: Vec<LayerId> = if split {
        targets.iter().map(|(_, t)| *t).collect()
    } else {
        vec![layer]
    };
    let frame = |f: u32| {
        Frame::new(IRect::new(at.x + (f * w) as i32, at.y, w, h))
            .with_duration(ase.durations.get(f as usize).copied().unwrap_or(100) as f32 / 1000.0)
    };
    let mut sprites = Vec::new();
    if ase.tags.is_empty() {
        let mut frames: Vec<Frame> = (0..ase.frames.len() as u32).map(frame).collect();
        if frames.len() == 1 {
            frames[0].duration = 0.0;
        }
        sprites.push(Sprite::new(name, layers.clone(), frames));
    } else {
        for (tag, from, to) in &ase.tags {
            let mut frames: Vec<Frame> = (*from..=*to).map(frame).collect();
            if frames.len() == 1 {
                frames[0].duration = 0.0;
            }
            sprites.push(Sprite::new(format!("{name}/{}", snake(tag)), layers.clone(), frames));
        }
    }
    let names = sprites.iter().map(|s| s.name.clone()).collect();
    for s in sprites {
        project.sprites.retain(|old| old.name != s.name);
        project.sprites.push(s);
    }
    Ok(names)
}

fn to_image(px: &[[u8; 4]], w: u32, h: u32) -> RgbaImage {
    RgbaImage {
        w,
        h,
        px: px.iter().flatten().copied().collect(),
    }
}

/// `WallSlideArmed` → `wall_slide_armed`, `Run 2` → `run_2`.
pub fn snake(s: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_uppercase() {
            if prev_lower {
                out.push('_');
            }
            out.extend(c.to_lowercase());
            prev_lower = false;
        } else if c.is_alphanumeric() {
            out.push(c);
            prev_lower = c.is_lowercase() || c.is_numeric();
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
            prev_lower = false;
        }
    }
    out.trim_end_matches('_').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_parse() {
        let hex = parse_hex("ff0000\n#00ff00\n\n").unwrap();
        assert_eq!(hex[1].1, [0, 255, 0, 255]);
        let gpl = parse_gpl("GIMP Palette\nName: t\n#\n 10  20  30 dark thing\n255 255 255\n").unwrap();
        assert_eq!(gpl[0], ("dark thing".into(), [10, 20, 30, 255]));
        assert_eq!(gpl[1].0, "#ffffff");
    }

    #[test]
    fn names_become_snake_case() {
        assert_eq!(snake("WallSlideArmed"), "wall_slide_armed");
        assert_eq!(snake("Run 2"), "run_2");
        assert_eq!(snake("idle"), "idle");
    }

    #[test]
    fn unmatched_colours_are_added_once() {
        let mut p = Project::new("t");
        p.palette.begin_section("A");
        p.palette.push("red", [255, 0, 0, 255]).unwrap();
        let l = p.add_layer("art", LayerKind::Pixels(Default::default()));
        let img = RgbaImage {
            w: 3,
            h: 1,
            px: vec![255, 0, 0, 255, 0, 0, 255, 255, 0, 0, 255, 255],
        };
        let mut r = Report::default();
        paint_rgba(&mut p, l, IVec2::ZERO, &img, None, &mut r).unwrap();
        assert_eq!(r.added, vec![[0, 0, 255, 255]]);
        assert_eq!(p.palette.section("Imported").unwrap().len, 1);
        assert_eq!(p.layer(l).unwrap().pixels().unwrap().get(IVec2::new(2, 0)), 2);
    }
}
