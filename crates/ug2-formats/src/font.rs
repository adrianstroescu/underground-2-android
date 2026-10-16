//! Bitmap fonts (chunk 0x30201, "FNTF"): in `LANGUAGES/<Language>.bin` and
//! `GLOBAL/GLOBALA.BUN`. The glyph images are textures named after the font (upper-case)
//! in `LANGUAGES/LanguageTextures.bin` / `GLOBALA.BUN`, white with the glyphs in alpha.
//!
//! Chunk: a 0x100-byte name block ("conduitmditc_tt14i", "bin", path), a second name
//! block, then FNTF:
//! * +0x0A u16 glyph count, +0x12 u16 line height?, +0x14 u32 glyph table offset,
//!   +0x18 u32 kerning table offset, +0x1C u32 end of kerning, +0x24 u32 ascent?,
//!   +0x40 f32 1 / texture width, +0x44 f32 1 / texture height.
//! * Glyphs, 16 bytes: u16 char, u8 width, u8 height, u16 u, u16 v (texture pixels),
//!   i8 x offset, i8 y offset, u16 ?, u16 ?, u16 advance.
//! * Kerning, 4 bytes: u16 second char, i8 adjustment, u8 first char.

use std::collections::HashMap;

use crate::bytes::{f32_at, u16_at, u32_at};
use crate::chunk::ChunkIter;

const FONT: u32 = 0x0003_0201;

#[derive(Clone, Copy, Debug, Default)]
pub struct Glyph {
    pub width: u8,
    pub height: u8,
    pub u: u16,
    pub v: u16,
    pub x_offset: i8,
    pub y_offset: i8,
    pub advance: u16,
}

#[derive(Clone, Debug, Default)]
pub struct Font {
    /// Lower-case name, e.g. "conduitmditc_tt14i"; the texture is its upper-case form.
    pub name: String,
    pub line_height: u16,
    pub ascent: u32,
    pub texture_size: (f32, f32),
    pub glyphs: HashMap<u16, Glyph>,
    pub kerning: HashMap<(u16, u16), i8>,
}

impl Font {
    pub fn texture_name(&self) -> String {
        self.name.to_ascii_uppercase()
    }

    pub fn glyph(&self, c: char) -> Option<&Glyph> {
        self.glyphs.get(&(c as u32 as u16))
    }

    pub fn kern(&self, a: char, b: char) -> i32 {
        self.kerning.get(&(a as u32 as u16, b as u32 as u16)).copied().unwrap_or(0) as i32
    }

    /// Width of `text` in font pixels.
    pub fn width(&self, text: &str) -> i32 {
        let mut w = 0;
        let mut prev = None;
        for c in text.chars() {
            if let Some(p) = prev {
                w += self.kern(p, c);
            }
            w += self.glyph(c).map_or(0, |g| g.advance as i32);
            prev = Some(c);
        }
        w
    }

    fn parse(chunk: &[u8]) -> Option<Font> {
        let name_end = chunk.iter().position(|&b| b == 0)?;
        let name = String::from_utf8_lossy(&chunk[..name_end]).to_ascii_lowercase();
        let f = chunk.get(0x200..)?;
        if f.get(0..4)? != b"FNTF" {
            return None;
        }
        let count = u16_at(f, 0x0A).ok()? as usize;
        let table = u32_at(f, 0x14).ok()? as usize;
        let kern = u32_at(f, 0x18).ok()? as usize;
        let kern_end = u32_at(f, 0x1C).ok()? as usize;
        let (iw, ih) = (f32_at(f, 0x40).ok()?, f32_at(f, 0x44).ok()?);
        let mut font = Font {
            name,
            line_height: u16_at(f, 0x12).ok()?,
            ascent: u32_at(f, 0x24).ok()?,
            texture_size: (if iw > 0.0 { 1.0 / iw } else { 256.0 }, if ih > 0.0 { 1.0 / ih } else { 256.0 }),
            ..Default::default()
        };
        for i in 0..count {
            let g = f.get(table + i * 16..table + i * 16 + 16)?;
            let c = u16::from_le_bytes([g[0], g[1]]);
            font.glyphs.insert(
                c,
                Glyph {
                    width: g[2],
                    height: g[3],
                    u: u16::from_le_bytes([g[4], g[5]]),
                    v: u16::from_le_bytes([g[6], g[7]]),
                    x_offset: g[8] as i8,
                    y_offset: g[9] as i8,
                    advance: u16::from_le_bytes([g[14], g[15]]),
                },
            );
        }
        let mut o = kern;
        while o + 4 <= kern_end.min(f.len()) {
            let second = u16::from_le_bytes([f[o], f[o + 1]]);
            font.kerning.insert((f[o + 3] as u16, second), f[o + 2] as i8);
            o += 4;
        }
        Some(font)
    }
}

/// Every font in a file (a language file or GLOBALA).
pub fn fonts(file: &[u8]) -> Vec<Font> {
    ChunkIter::new(file).filter(|c| c.id == FONT).filter_map(|c| Font::parse(c.data)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_fonts() {
        let game = crate::fs::GameDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        let Ok(file) = game.read("LANGUAGES/English.bin.bak").or_else(|_| game.read("LANGUAGES/English.bin")) else { return };
        let fonts = fonts(&file);
        let names: Vec<&str> = fonts.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"conduitmditc_tt14i"), "{names:?}");
        let f = fonts.iter().find(|f| f.name == "conduitmditc_tt14i").unwrap();
        assert_eq!(f.texture_size, (256.0, 128.0));
        let a = f.glyph('A').unwrap();
        assert!(a.width > 4 && a.height > 8 && a.advance > 4);
        assert!(f.width("QUICK RACE") > 50);
    }
}
