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
