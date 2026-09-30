//! The one palette every pixel in a project indexes into.
//!
//! Index 0 is always fully transparent and is what an unpainted pixel holds. Everything else is an
//! RGBA entry with a name, and entries are grouped into labelled [`Section`]s — "DawnBringer 32",
//! "Clothing", "UI" — that are contiguous ranges of indices. Two entries may hold the same colour
//! on purpose: a player's shoe tone and the earth of a wall are both `#8f563b`, but recolouring
//! one must never touch the other.

use serde::{Deserialize, Serialize};

pub type Rgba = [u8; 4];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaletteEntry {
    pub name: String,
    pub rgba: Rgba,
}

/// A labelled, contiguous run of palette indices: `start..start + len`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub label: String,
    pub start: u16,
    pub len: u16,
}

impl Section {
    pub fn indices(&self) -> impl Iterator<Item = u8> + '_ {
        (self.start..self.start + self.len).map(|i| i as u8)
    }

    pub fn contains(&self, index: u8) -> bool {
        (self.start..self.start + self.len).contains(&(index as u16))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Palette {
    entries: Vec<PaletteEntry>,
    sections: Vec<Section>,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            entries: vec![PaletteEntry {
                name: "transparent".into(),
                rgba: [0, 0, 0, 0],
            }],
            sections: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteFull;

impl std::fmt::Display for PaletteFull {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the palette already holds 256 entries")
    }
}

impl std::error::Error for PaletteFull {}

impl Palette {
    pub const TRANSPARENT: u8 = 0;

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.len() <= 1
    }

    pub fn entries(&self) -> &[PaletteEntry] {
        &self.entries
    }

    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    pub fn entry(&self, index: u8) -> Option<&PaletteEntry> {
        self.entries.get(index as usize)
    }

    pub fn entry_mut(&mut self, index: u8) -> Option<&mut PaletteEntry> {
        self.entries.get_mut(index as usize)
    }

    /// The colour of an index; out-of-range indices read as transparent.
    pub fn rgba(&self, index: u8) -> Rgba {
        self.entries
            .get(index as usize)
            .map_or([0; 4], |entry| entry.rgba)
    }

    /// Start a new section. Entries [`push`](Self::push)ed after this belong to it.
    pub fn begin_section(&mut self, label: impl Into<String>) {
        self.sections.push(Section {
            label: label.into(),
            start: self.entries.len() as u16,
            len: 0,
        });
    }

    /// Append an entry to the last section begun (or to no section, if none was).
    pub fn push(&mut self, name: impl Into<String>, rgba: Rgba) -> Result<u8, PaletteFull> {
        if self.entries.len() >= 256 {
            return Err(PaletteFull);
        }
        let index = self.entries.len() as u8;
        self.entries.push(PaletteEntry {
            name: name.into(),
            rgba,
        });
        if let Some(section) = self.sections.last_mut()
            && section.start + section.len == index as u16
        {
            section.len += 1;
        }
        Ok(index)
    }

    /// Insert an entry at the end of the `section`-th section and return its index. Every entry at
    /// or after that index moves up one, so anything holding indices must be remapped — use
    /// [`Project::insert_color`](crate::Project::insert_color), which does.
    pub(crate) fn insert_into_section(&mut self, section: usize, name: String, rgba: Rgba) -> Result<u8, PaletteFull> {
        if self.entries.len() >= 256 {
            return Err(PaletteFull);
        }
        let at = self.sections[section].start + self.sections[section].len;
        self.entries.insert(at as usize, PaletteEntry { name, rgba });
        self.sections[section].len += 1;
        for s in self.sections.iter_mut().skip(section + 1) {
            if s.start >= at {
                s.start += 1;
            }
        }
        Ok(at as u8)
    }

    /// Remove an entry. Everything after it moves down one; see
    /// [`Project::remove_color`](crate::Project::remove_color).
    pub(crate) fn remove(&mut self, index: u8) {
        let at = index as u16;
        if at == 0 || at as usize >= self.entries.len() {
            return;
        }
        self.entries.remove(at as usize);
        for s in &mut self.sections {
            if s.contains(index) {
                s.len -= 1;
            } else if s.start > at {
                s.start -= 1;
            }
        }
    }

    pub fn rename_section(&mut self, section: usize, label: impl Into<String>) {
        if let Some(s) = self.sections.get_mut(section) {
            s.label = label.into();
        }
    }

    /// Forget sections that ended up holding nothing.
    pub fn drop_empty_sections(&mut self) {
        self.sections.retain(|s| s.len > 0);
    }

    pub fn section(&self, label: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.label == label)
    }

    /// Which section an index belongs to.
    pub fn section_of(&self, index: u8) -> Option<&Section> {
        self.sections.iter().find(|s| s.contains(index))
    }

    /// The first entry of exactly this colour, looking only in `section` if one is given.
    /// Never matches the transparent entry.
    pub fn find(&self, rgba: Rgba, section: Option<&str>) -> Option<u8> {
        let candidates: Box<dyn Iterator<Item = u8>> = match section {
            Some(label) => Box::new(self.section(label)?.indices()),
            None => Box::new(1..self.entries.len() as u8),
        };
        candidates
            .into_iter()
            .find(|&i| self.entries[i as usize].rgba == rgba)
    }

    /// The entry named `name` within `section`.
    pub fn named(&self, section: &str, name: &str) -> Option<u8> {
        self.section(section)?
            .indices()
            .find(|&i| self.entries[i as usize].name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_track_their_entries() {
        let mut p = Palette::default();
        p.begin_section("A");
        let a = p.push("red", [255, 0, 0, 255]).unwrap();
        p.push("green", [0, 255, 0, 255]).unwrap();
        p.begin_section("B");
        let b = p.push("red again", [255, 0, 0, 255]).unwrap();
        assert_eq!(p.section("A").unwrap().len, 2);
        assert_eq!(p.find([255, 0, 0, 255], None), Some(a));
        assert_eq!(p.find([255, 0, 0, 255], Some("B")), Some(b));
        assert_eq!(p.named("B", "red again"), Some(b));
        assert_eq!(p.section_of(b).unwrap().label, "B");
        assert_eq!(p.find([0, 0, 0, 0], None), None);
    }
}

/// Well-known palettes, for starting a project.
pub mod presets {
    /// `(name, colours as 0xRRGGBB)`.
    pub type Preset = (&'static str, &'static [u32]);

    pub const DB32: Preset = (
        "DawnBringer 32",
        &[
            0x000000, 0x222034, 0x45283c, 0x663931, 0x8f563b, 0xdf7126, 0xd9a066, 0xeec39a, 0xfbf236,
            0x99e550, 0x6abe30, 0x37946e, 0x4b692f, 0x524b24, 0x323c39, 0x3f3f74, 0x306082, 0x5b6ee1,
            0x639bff, 0x5fcde4, 0xcbdbfc, 0xffffff, 0x9badb7, 0x847e87, 0x696a6a, 0x595652, 0x76428a,
            0xac3232, 0xd95763, 0xd77bba, 0x8f974a, 0x8a6f30,
        ],
    );

    pub const PICO8: Preset = (
        "PICO-8",
        &[
            0x000000, 0x1d2b53, 0x7e2553, 0x008751, 0xab5236, 0x5f574f, 0xc2c3c7, 0xfff1e8, 0xff004d,
            0xffa300, 0xffec27, 0x00e436, 0x29adff, 0x83769c, 0xff77a8, 0xffccaa,
        ],
    );

    pub const ENDESGA32: Preset = (
        "Endesga 32",
        &[
            0xbe4a2f, 0xd77643, 0xead4aa, 0xe4a672, 0xb86f50, 0x733e39, 0x3e2731, 0xa22633, 0xe43b44,
            0xf77622, 0xfeae34, 0xfee761, 0x63c74d, 0x3e8948, 0x265c42, 0x193c3e, 0x124e89, 0x0099db,
            0x2ce8f5, 0xffffff, 0xc0cbdc, 0x8b9bb4, 0x5a6988, 0x3a4466, 0x262b44, 0x181425, 0xff0044,
            0x68386c, 0xb55088, 0xf6757a, 0xe8b796, 0xc28569,
        ],
    );

    pub const ALL: [Preset; 3] = [DB32, PICO8, ENDESGA32];

    /// Append a preset as its own section.
    pub fn add(palette: &mut super::Palette, preset: Preset) -> Result<(), super::PaletteFull> {
        palette.begin_section(preset.0);
        for &v in preset.1 {
            palette.push(format!("#{v:06x}"), [(v >> 16) as u8, (v >> 8) as u8, v as u8, 255])?;
        }
        Ok(())
    }
}
