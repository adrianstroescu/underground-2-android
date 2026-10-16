//! Shared tuning parts: rims (`CARS/WHEELS/GEOMETRY_<BRAND>.BIN`) and spoilers
//! (`CARS/SPOILER*`), catalogued once and loaded on demand on the loader thread.
//!
//! Rims include the tyre and are modelled in the same frame as a car's stock front
//! wheel, so one replaces the other; each style comes in several sizes, and the one
//! closest to the car's wheel radius is used. Spoilers are modelled at their own origin
//! and placed on the `SPOILER` marker of the car's `<CAR>_BASE` part.

use std::collections::HashMap;

use crate::fs::GameDir;
use crate::solid::{find_solid_lists, Solid};
use crate::tpk::{find_tpks, Texture};

/// Rim brand files in CARS/WHEELS.
const RIM_FILES: &[&str] = &[
    "5ZIGEN", "ADVAN", "AVUS", "BBS", "DAVIN", "DONZ", "ENKEI", "FOXX", "GIANELLE", "GIOVANNA", "KAIZER", "KONIG",
    "LEXANI", "LOWENHART", "MOMO", "NFSU", "OASIS", "OZ", "RACINGHART", "ROTA", "VOLK", "WELDWHEEL", "WORK",
];

/// Spoiler marker in `<CAR>_BASE`.
pub const SPOILER_MARKER: u32 = 0xC93B_73FD;
pub const SPOILER_STYLES: u8 = 40;

#[derive(Clone, Debug)]
pub struct RimEntry {
    pub file: String,
    /// Solid name prefix, e.g. "BBS_STYLE03_".
    pub prefix: String,
    /// Menu label, e.g. "BBS 03".
    pub label: String,
}

#[derive(Default)]
pub struct PartLibrary {
    pub rims: Vec<RimEntry>,
    files: HashMap<String, Vec<Solid>>,
    /// Textures of rims and spoilers.
    pub textures: HashMap<u32, Texture>,
}

fn style_prefix(name: &str) -> Option<String> {
    // "BBS_STYLE03_17_24_A" -> "BBS_STYLE03_"
    let at = name.find("_STYLE")?;
    let end = at + "_STYLE".len() + 2;
    (name.len() > end).then(|| format!("{}_", &name[..end]))
}

impl PartLibrary {
    /// Lists every rim style (parsing the brand files once) and loads part textures.
    pub fn scan(game: &GameDir) -> Self {
        let mut lib = Self::default();
        for file in RIM_FILES {
            let Ok(data) = game.read(&format!("CARS/WHEELS/GEOMETRY_{file}.BIN")) else { continue };
            let mut seen: Vec<String> = Vec::new();
            for s in find_solid_lists(&data).into_iter().flat_map(|l| l.solids) {
                if let Some(p) = style_prefix(&s.name) {
                    if !seen.contains(&p) {
                        seen.push(p);
                    }
                }
            }
            seen.sort();
            for prefix in seen {
                let label = prefix.trim_end_matches('_').replace("_STYLE", " ");
                lib.rims.push(RimEntry { file: file.to_string(), prefix, label });
            }
        }
        for tex in ["CARS/WHEELS/TEXTURES.BIN", "CARS/SPOILER/TEXTURES.BIN", "CARS/SPOILER_HATCH/TEXTURES.BIN", "CARS/SPOILER_SUV/TEXTURES.BIN"] {
            if let Ok(d) = game.read(tex) {
                for t in find_tpks(&d).into_iter().flat_map(|p| p.textures) {
                    lib.textures.insert(t.hash, t);
                }
            }
        }
        log::info!("parts: {} rim styles, {} part textures", lib.rims.len(), lib.textures.len());
        lib
    }

    fn file(&mut self, game: &GameDir, rel: &str) -> &[Solid] {
        self.files
            .entry(rel.to_string())
            .or_insert_with(|| {
                game.read(rel).map(|d| find_solid_lists(&d).into_iter().flat_map(|l| l.solids).collect()).unwrap_or_default()
            })
            .as_slice()
    }

    /// Rim `index` (1-based into [`PartLibrary::rims`]) in the size closest to `radius`.
    pub fn rim(&mut self, game: &GameDir, index: u16, radius: f32) -> Option<Solid> {
        let entry = self.rims.get(index.checked_sub(1)? as usize)?.clone();
        let solids = self.file(game, &format!("CARS/WHEELS/GEOMETRY_{}.BIN", entry.file));
        solids
            .iter()
            .filter(|s| s.name.starts_with(&entry.prefix) && s.name.ends_with("_A") && s.mesh.is_some())
            .min_by(|a, b| {
                let r = |s: &Solid| (s.bbox_max[2] - s.bbox_min[2]) * 0.5;
                (r(a) - radius).abs().total_cmp(&(r(b) - radius).abs())
            })
            .cloned()
    }

    /// Spoiler `style` (1..=40), carbon or painted, from the set that suits `car`.
    pub fn spoiler(&mut self, game: &GameDir, car: &str, style: u8, carbon: bool) -> Option<Solid> {
        let folder = match car {
            "HUMMER" | "NAVIGATOR" | "ESCALADE" => "SPOILER_SUV",
            "PEUGOT" | "PEUGOT106" | "CORSA" | "FOCUS" | "GOLF" | "CIVIC" | "A3" => "SPOILER_HATCH",
            _ => "SPOILER",
        };
        let name = if carbon { format!("SPOILER_STYLE{style:02}_CF_A") } else { format!("SPOILER_STYLE{style:02}_A") };
        let hash = crate::bin_hash(&name);
        self.file(game, &format!("CARS/{folder}/GEOMETRY.BIN")).iter().find(|s| s.hash == hash).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rims_and_spoilers_from_the_game() {
        let game = GameDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        if game.read("CARS/WHEELS/GEOMETRY_BBS.BIN").is_err() {
            return;
        }
        let mut lib = PartLibrary::scan(&game);
        assert!(lib.rims.len() > 100, "{} rim styles", lib.rims.len());
        let rim = lib.rim(&game, 1, 0.33).expect("first rim");
        assert!(rim.mesh.is_some());
        let r = (rim.bbox_max[2] - rim.bbox_min[2]) * 0.5;
        assert!((0.25..0.45).contains(&r), "rim radius {r}");
        assert!(lib.spoiler(&game, "350Z", 1, false).is_some());
        assert!(lib.spoiler(&game, "350Z", 40, true).is_some());
        // The 350Z base part carries the spoiler mount.
        let geo = game.read("CARS/350Z/GEOMETRY.BIN").unwrap();
        let base = crate::bin_hash("350Z_BASE_A");
        let solids: Vec<Solid> = find_solid_lists(&geo).into_iter().flat_map(|l| l.solids).collect();
        let m = solids.iter().find(|s| s.hash == base).unwrap().markers.iter().find(|m| m.hash == SPOILER_MARKER).copied();
        let p = m.expect("spoiler marker").position;
        assert!(p[0] < -1.5 && p[2] > 0.5, "marker at {p:?}");
    }
}
