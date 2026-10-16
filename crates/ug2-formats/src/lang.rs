//! Localised strings (`LANGUAGES/<Language>.bin`): chunk 0x39000 holding a table of
//! (label hash, string offset) pairs. Labels are hashed with `bin_hash` (e.g.
//! "SMS_CAR_UNLOCK_1_BODY"); the frontend packages refer to strings by that hash.
//!
//! Body: u32 header size (16), u32 count, u32 table offset, u32 strings offset (both from
//! the body start), then the table and NUL-terminated strings in the game's 8-bit
//! encoding (Latin-1 for the Western languages).

use std::collections::HashMap;

use crate::bytes::u32_at;
use crate::chunk::ChunkIter;
use crate::hash::bin_hash;

const LANGUAGE: u32 = 0x0003_9000;

#[derive(Clone, Debug, Default)]
pub struct Strings {
    map: HashMap<u32, String>,
}

impl Strings {
    pub fn parse(file: &[u8]) -> Self {
        let mut out = Self::default();
        for c in ChunkIter::new(file).filter(|c| c.id == LANGUAGE) {
            let p = c.data;
            let (Ok(count), Ok(table), Ok(strings)) = (u32_at(p, 4), u32_at(p, 8), u32_at(p, 12)) else { continue };
            for i in 0..count as usize {
                let (Ok(hash), Ok(off)) = (u32_at(p, table as usize + i * 8), u32_at(p, table as usize + i * 8 + 4)) else { break };
                let start = strings as usize + off as usize;
                let Some(rest) = p.get(start..) else { continue };
                let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
                out.map.insert(hash, rest[..end].iter().map(|&b| b as char).collect());
            }
        }
        out
    }

    pub fn by_hash(&self, hash: u32) -> Option<&str> {
        self.map.get(&hash).map(String::as_str)
    }

    pub fn get(&self, label: &str) -> Option<&str> {
        self.by_hash(bin_hash(label))
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english() {
        let game = crate::fs::GameDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        let Ok(file) = game.read("LANGUAGES/English.bin.bak").or_else(|_| game.read("LANGUAGES/English.bin")) else { return };
        let s = Strings::parse(&file);
        assert!(s.len() > 3000);
        assert_eq!(s.get("GO"), Some("GO!"));
    }
}
