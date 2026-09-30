//! Reading and writing PNGs, for importing art and exporting previews.

use std::io::Cursor;

use crate::palette::{Palette, Rgba};
use crate::render::{Indexed, RgbaImage};

pub enum Decoded {
    /// An 8-bit palette PNG: its own palette (with `tRNS` alpha applied) and indices into it.
    Indexed { palette: Vec<Rgba>, image: Indexed },
    Rgba(RgbaImage),
}

impl Decoded {
    pub fn size(&self) -> (u32, u32) {
        match self {
            Decoded::Indexed { image, .. } => (image.w, image.h),
            Decoded::Rgba(img) => (img.w, img.h),
        }
    }

    /// The picture as straight RGBA, whatever it was stored as.
    pub fn to_rgba(&self) -> RgbaImage {
        match self {
            Decoded::Rgba(img) => img.clone(),
            Decoded::Indexed { palette, image } => {
                let mut out = RgbaImage::new(image.w, image.h);
                for (i, &v) in image.px.iter().enumerate() {
                    let c = palette.get(v as usize).copied().unwrap_or([0; 4]);
                    out.px[i * 4..i * 4 + 4].copy_from_slice(&c);
                }
                out
            }
        }
    }
}

pub fn decode(bytes: &[u8]) -> Result<Decoded, png::DecodingError> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let header = decoder.read_header_info()?;
    let indexed8 =
        header.color_type == png::ColorType::Indexed && header.bit_depth == png::BitDepth::Eight;
    if !indexed8 {
        return decode_rgba(bytes).map(Decoded::Rgba);
    }
    let mut reader = decoder.read_info()?;
    let info = reader.info();
    let (w, h) = (info.width, info.height);
    let plte = info.palette.as_deref().unwrap_or(&[]).to_vec();
    let trns = info.trns.as_deref().unwrap_or(&[]).to_vec();
    let palette = plte
        .chunks_exact(3)
        .enumerate()
        .map(|(i, c)| [c[0], c[1], c[2], trns.get(i).copied().unwrap_or(255)])
        .collect();
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or((w * h) as usize)];
    let frame = reader.next_frame(&mut buf)?;
    let mut image = Indexed::new(w, h);
    for y in 0..h as usize {
        let row = &buf[y * frame.line_size..y * frame.line_size + w as usize];
        image.px[y * w as usize..(y + 1) * w as usize].copy_from_slice(row);
    }
    Ok(Decoded::Indexed { palette, image })
}

fn decode_rgba(bytes: &[u8]) -> Result<RgbaImage, png::DecodingError> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let frame = reader.next_frame(&mut buf)?;
    let (w, h) = (frame.width, frame.height);
    let mut out = RgbaImage::new(w, h);
    let channels = frame.color_type.samples();
    for y in 0..h as usize {
        for x in 0..w as usize {
            let s = &buf[y * frame.line_size + x * channels..][..channels];
            let px = match channels {
                1 => [s[0], s[0], s[0], 255],
                2 => [s[0], s[0], s[0], s[1]],
                3 => [s[0], s[1], s[2], 255],
                _ => [s[0], s[1], s[2], s[3]],
            };
            let d = (y * w as usize + x) * 4;
            out.px[d..d + 4].copy_from_slice(&px);
        }
    }
    Ok(out)
}

pub fn encode_rgba(img: &RgbaImage) -> Result<Vec<u8>, png::EncodingError> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.w, img.h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(&img.px)?;
    }
    Ok(out)
}

/// An 8-bit palette PNG carrying the project palette, so tools that understand indexed PNGs see
/// the real indices.
pub fn encode_indexed(img: &Indexed, palette: &Palette) -> Result<Vec<u8>, png::EncodingError> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.w, img.h);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::Eight);
        let entries = palette.entries();
        enc.set_palette(entries.iter().flat_map(|e| [e.rgba[0], e.rgba[1], e.rgba[2]]).collect::<Vec<_>>());
        enc.set_trns(entries.iter().map(|e| e.rgba[3]).collect::<Vec<_>>());
        enc.write_header()?.write_image_data(&img.px)?;
    }
    Ok(out)
}
