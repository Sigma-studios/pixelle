//! Just enough of the `.aseprite` format to import art: normal layers, raw, compressed and linked
//! cels, both palette chunks, tags and frame durations. Group layers are skipped with their
//! children flattened in; tilemap layers, blend modes and opacity are not read.
//!
//! Format reference: <https://github.com/aseprite/aseprite/blob/main/docs/ase-file-specs.md>

use std::fmt;

/// Why a file could not be read.
#[derive(Debug)]
pub struct AseError(pub String);

impl fmt::Display for AseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for AseError {}

type Result<T> = std::result::Result<T, AseError>;

macro_rules! bail {
    ($($t:tt)*) => { return Err(AseError(format!($($t)*))) };
}

macro_rules! ensure {
    ($c:expr, $($t:tt)*) => { if !$c { bail!($($t)*) } };
}

trait Context<T> {
    fn context(self, msg: &str) -> Result<T>;
}

impl<T> Context<T> for Option<T> {
    fn context(self, msg: &str) -> Result<T> {
        self.ok_or_else(|| AseError(msg.into()))
    }
}

pub struct Ase {
    pub width: u32,
    pub height: u32,
    pub layers: Vec<AseLayer>,
    pub tags: Vec<(String, u32, u32)>,
    /// Milliseconds each frame shows for.
    pub durations: Vec<u32>,
    /// `frames[f][layer]`: that layer's picture in that frame as straight RGBA, `width * height`.
    pub frames: Vec<Vec<Vec<[u8; 4]>>>,
}

pub struct AseLayer {
    pub name: String,
    pub visible: bool,
    /// A group holds no pixels of its own; its children follow it.
    pub group: bool,
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self.b.get(self.at..self.at + n).context("file ends early")?;
        self.at += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn string(&mut self) -> Result<String> {
        let n = self.u16()? as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
}

struct Cel {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    /// Raw pixels in the file's colour depth.
    data: Vec<u8>,
}

pub fn parse(bytes: &[u8]) -> Result<Ase> {
    let mut r = Reader { b: bytes, at: 0 };
    r.u32()?;
    ensure!(r.u16()? == 0xA5E0, "not an .aseprite file");
    let frame_count = r.u16()? as usize;
    let width = r.u16()? as u32;
    let height = r.u16()? as u32;
    let depth = r.u16()?;
    ensure!(depth == 8 || depth == 32, "colour depth {depth} is not supported");
    r.take(14)?;
    let transparent = r.u8()?;
    r.at = 128;

    let mut layers = Vec::new();
    let mut tags = Vec::new();
    let mut durations = Vec::new();
    let mut palette = vec![[0u8; 4]; 256];
    // Cels as stored, `None` where a cel links to an earlier frame's.
    let mut cels: Vec<Vec<Option<Cel>>> = Vec::new();
    let mut links: Vec<(usize, usize, usize)> = Vec::new();

    for f in 0..frame_count {
        let start = r.at;
        let size = r.u32()? as usize;
        ensure!(r.u16()? == 0xF1FA, "bad frame header");
        let old_chunks = r.u16()? as usize;
        durations.push(r.u16()? as u32);
        r.take(2)?;
        let new_chunks = r.u32()? as usize;
        let chunks = if new_chunks == 0 { old_chunks } else { new_chunks };
        cels.push(Vec::new());
        for _ in 0..chunks {
            let chunk_start = r.at;
            let chunk_size = r.u32()? as usize;
            let kind = r.u16()?;
            match kind {
                0x2004 => {
                    let flags = r.u16()?;
                    let layer_type = r.u16()?;
                    // Child level, default size, blend mode, opacity, reserved.
                    r.take(12)?;
                    let name = r.string()?;
                    if layer_type == 2 {
                        bail!("layer {name:?} is a tilemap, which this reader does not support");
                    }
                    layers.push(AseLayer {
                        name,
                        visible: flags & 1 != 0,
                        group: layer_type == 1,
                    });
                }
                0x2005 => {
                    let layer = r.u16()? as usize;
                    let x = r.i16()? as i32;
                    let y = r.i16()? as i32;
                    r.u8()?;
                    let cel_type = r.u16()?;
                    r.take(7)?;
                    let frame_cels = cels.last_mut().expect("pushed above");
                    if frame_cels.len() <= layer {
                        frame_cels.resize_with(layer + 1, || None);
                    }
                    let bpp = depth as usize / 8;
                    match cel_type {
                        0 | 2 => {
                            let w = r.u16()? as u32;
                            let h = r.u16()? as u32;
                            let rest = chunk_start + chunk_size - r.at;
                            let raw = r.take(rest)?;
                            let data = if cel_type == 2 {
                                miniz_oxide::inflate::decompress_to_vec_zlib(raw)
                                    .map_err(|e| AseError(format!("bad cel data: {e:?}")))?
                            } else {
                                raw.to_vec()
                            };
                            ensure!(data.len() >= (w * h) as usize * bpp, "cel is short");
                            frame_cels[layer] = Some(Cel { x, y, w, h, data });
                        }
                        1 => links.push((f, layer, r.u16()? as usize)),
                        other => bail!("cel type {other} is not supported"),
                    }
                }
                0x2018 => {
                    let n = r.u16()?;
                    r.take(8)?;
                    for _ in 0..n {
                        let from = r.u16()? as u32;
                        let to = r.u16()? as u32;
                        r.take(13)?;
                        tags.push((r.string()?, from, to));
                    }
                }
                // The old palette chunk, which files that never needed alpha still carry alone.
                0x0004 => {
                    let packets = r.u16()?;
                    let mut i = 0usize;
                    for _ in 0..packets {
                        i += r.u8()? as usize;
                        let n = match r.u8()? {
                            0 => 256,
                            n => n as usize,
                        };
                        for _ in 0..n {
                            let c = r.take(3)?;
                            if i < 256 {
                                palette[i] = [c[0], c[1], c[2], 255];
                            }
                            i += 1;
                        }
                    }
                }
                0x2019 => {
                    r.u32()?;
                    let first = r.u32()? as usize;
                    let last = r.u32()? as usize;
                    r.take(8)?;
                    for i in first..=last {
                        let flags = r.u16()?;
                        let c = r.take(4)?;
                        if i < 256 {
                            palette[i] = [c[0], c[1], c[2], c[3]];
                        }
                        if flags & 1 != 0 {
                            r.string()?;
                        }
                    }
                }
                _ => {}
            }
            r.at = chunk_start + chunk_size;
        }
        r.at = start + size;
    }

    // Flatten each cel to a full-canvas RGBA picture per layer.
    let to_rgba = |data: &[u8], i: usize| -> [u8; 4] {
        if depth == 32 {
            [data[i * 4], data[i * 4 + 1], data[i * 4 + 2], data[i * 4 + 3]]
        } else {
            let v = data[i];
            if v == transparent {
                [0; 4]
            } else {
                palette[v as usize]
            }
        }
    };
    let blank = vec![[0u8; 4]; (width * height) as usize];
    let picture = |cel: &Cel| {
        let mut out = blank.clone();
        for cy in 0..cel.h as i32 {
            for cx in 0..cel.w as i32 {
                let (x, y) = (cel.x + cx, cel.y + cy);
                if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                    continue;
                }
                out[(y as u32 * width + x as u32) as usize] =
                    to_rgba(&cel.data, (cy as u32 * cel.w + cx as u32) as usize);
            }
        }
        out
    };
    let mut frames: Vec<Vec<Vec<[u8; 4]>>> = cels
        .iter()
        .map(|frame| {
            (0..layers.len())
                .map(|l| frame.get(l).and_then(|c| c.as_ref()).map_or(blank.clone(), &picture))
                .collect()
        })
        .collect();
    for (f, layer, from) in links {
        frames[f][layer] = frames[from][layer].clone();
    }

    Ok(Ase {
        width,
        height,
        layers,
        tags,
        durations,
        frames,
    })
}

impl Ase {
    /// The visible layers of a frame, stacked bottom to top (opaque over; no blending needed for
    /// pixel art whose layers do not overlap translucently).
    pub fn visible(&self, frame: usize) -> Vec<[u8; 4]> {
        let mut out = vec![[0u8; 4]; (self.width * self.height) as usize];
        for (l, layer) in self.layers.iter().enumerate() {
            if !layer.visible || layer.group {
                continue;
            }
            for (o, p) in out.iter_mut().zip(&self.frames[frame][l]) {
                if p[3] != 0 {
                    *o = *p;
                }
            }
        }
        out
    }
}
