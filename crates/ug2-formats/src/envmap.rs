//! Environment parameter maps (chunk 0x8003b600 in a region index): values that change
//! across the city, such as the screen tint and the fog, stored as records picked by a
//! quadtree over the map.
//!
//! The game looks the record up at the camera each frame and eases towards it:
//! `current = lerp(current, record, min(1, camera moved / 50 m))` (speed2.exe 0x5fa660).

use crate::bytes::{f32_at, u16_at, u32_at};
use crate::chunk::ChunkIter;
use crate::hash::bin_hash;

const MAPS: u32 = 0x8003_b600;
const MAP: u32 = 0x8003_b601;
const HEADER: u32 = 0x0003_b602;
const TYPES: u32 = 0x0003_b603;
const OFFSETS: u32 = 0x0003_b604;
const RECORDS: u32 = 0x0003_b605;
const TREE: u32 = 0x0003_b608;

/// Distance the camera travels while a value moves fully to a new record (speed2.exe
/// 0x7a47f4).
pub const BLEND_DISTANCE: f32 = 50.0;

#[derive(Clone, Debug)]
pub struct ParamMap {
    pub hash: u32,
    pub bbox_min: [f32; 2],
    pub bbox_max: [f32; 2],
    /// Per field: 0 = f32, 1 = i32.
    pub types: Vec<u32>,
    pub offsets: Vec<u32>,
    pub record_size: usize,
    pub records: Vec<u8>,
    /// Quadtree nodes. A node whose first entry is 0 is a leaf holding a record index in
    /// its second entry; otherwise the entries are children, quadrant 2·(x ≥ mid) + (y ≥ mid).
    pub nodes: Vec<[u16; 4]>,
}

impl ParamMap {
    pub fn record_count(&self) -> usize {
        self.records.len().checked_div(self.record_size).unwrap_or(0)
    }

    /// Record index at a map position (record 0 outside the tree or for a map without one).
    pub fn record_at(&self, x: f32, y: f32) -> usize {
        if self.nodes.is_empty() {
            return 0;
        }
        let (mut x0, mut y0) = (self.bbox_min[0], self.bbox_min[1]);
        let (mut x1, mut y1) = (self.bbox_max[0], self.bbox_max[1]);
        let mut node = 0usize;
        for _ in 0..32 {
            let Some(n) = self.nodes.get(node) else { return 0 };
            if n[0] == 0 {
                return n[1] as usize;
            }
            let (mx, my) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
            let (qx, qy) = (x >= mx, y >= my);
            if qx { x0 = mx } else { x1 = mx }
            if qy { y0 = my } else { y1 = my }
            node = n[qx as usize * 2 + qy as usize] as usize;
        }
        0
    }

    /// The fields of record `r` as floats (integer fields converted).
    pub fn record(&self, r: usize) -> Vec<f32> {
        let base = r * self.record_size;
        (0..self.offsets.len())
            .map(|f| {
                let o = base + self.offsets[f] as usize;
                match self.types.get(f) {
                    Some(1) => u32_at(&self.records, o).map(|v| v as i32 as f32).unwrap_or(0.0),
                    _ => f32_at(&self.records, o).unwrap_or(0.0),
                }
            })
            .collect()
    }

    pub fn values_at(&self, x: f32, y: f32) -> Vec<f32> {
        self.record(self.record_at(x, y))
    }
}

/// All parameter maps of a region.
#[derive(Clone, Debug, Default)]
pub struct EnvMaps {
    pub maps: Vec<ParamMap>,
}

impl EnvMaps {
    pub fn parse(index: &[u8]) -> Self {
        let mut out = Self::default();
        let Some(root) = ChunkIter::new(index).find(|c| c.id == MAPS) else { return out };
        for m in root.children().filter(|c| c.id == MAP) {
            if let Some(map) = parse_map(m.children()) {
                out.maps.push(map);
            }
        }
        out
    }

    pub fn get(&self, name: &str) -> Option<&ParamMap> {
        let h = bin_hash(name);
        self.maps.iter().find(|m| m.hash == h)
    }
}

fn parse_map(children: ChunkIter<'_>) -> Option<ParamMap> {
    let mut map = ParamMap {
        hash: 0,
        bbox_min: [0.0; 2],
        bbox_max: [0.0; 2],
        types: Vec::new(),
        offsets: Vec::new(),
        record_size: 0,
        records: Vec::new(),
        nodes: Vec::new(),
    };
    let mut fields = 0usize;
    for c in children {
        let p = c.data;
        match c.id {
            // u32 hash, bbox x0 y0 x1 y1, u32 nodes, u32 records, u32 record size, u32 fields.
            HEADER => {
                map.hash = u32_at(p, 0).ok()?;
                map.bbox_min = [f32_at(p, 4).ok()?, f32_at(p, 8).ok()?];
                map.bbox_max = [f32_at(p, 12).ok()?, f32_at(p, 16).ok()?];
                map.record_size = u32_at(p, 28).ok()? as usize;
                fields = u32_at(p, 32).ok()? as usize;
            }
            TYPES => map.types = (0..p.len() / 4).filter_map(|i| u32_at(p, i * 4).ok()).collect(),
            OFFSETS => map.offsets = (0..p.len() / 4).filter_map(|i| u32_at(p, i * 4).ok()).collect(),
            RECORDS => map.records = p.to_vec(),
            TREE => {
                map.nodes = p
                    .chunks_exact(8)
                    .map(|n| std::array::from_fn(|k| u16_at(n, k * 2).unwrap_or(0)))
                    .collect()
            }
            _ => {}
        }
    }
    map.offsets.truncate(fields);
    map.types.truncate(fields);
    Some(map)
}

/// A value set that eases towards the record under the camera, as the game does.
#[derive(Clone, Debug, Default)]
pub struct Tracked {
    pub values: Vec<f32>,
    last: Option<[f32; 2]>,
}

impl Tracked {
    pub fn update(&mut self, map: &ParamMap, x: f32, y: f32) -> &[f32] {
        let target = map.values_at(x, y);
        let w = match self.last {
            Some([lx, ly]) if self.values.len() == target.len() => ((x - lx).hypot(y - ly) / BLEND_DISTANCE).min(1.0),
            _ => 1.0,
        };
        if self.values.len() != target.len() {
            self.values = target.clone();
        }
        for (i, (v, t)) in self.values.iter_mut().zip(&target).enumerate() {
            let lerped = *v * (1.0 - w) + t * w;
            // Integer fields are truncated back to integers (ftol).
            *v = if map.types.get(i) == Some(&1) { lerped.trunc() } else { lerped };
        }
        self.last = Some([x, y]);
        &self.values
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn city_hall_tint_and_fog() {
        let game = crate::fs::GameDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        let Ok(index) = game.read("TRACKS/L4RA.BUN") else { return };
        let maps = EnvMaps::parse(&index);
        assert!(maps.maps.len() >= 10);
        // The camera of the apitrace capture at City Hall: the game sent TintColor
        // (156, 160, 145)/255 × 0.2 with w = 0.19, fog from 10 m to at most 40 %.
        let (x, y) = (-1378.65, 214.29);
        let tint = maps.get("Sunrise Screen Tint").unwrap().values_at(x, y);
        assert_eq!(tint, vec![156.0, 160.0, 145.0, 0.75, 0.41, 0.7, 0.19]);
        let fog = maps.get("Normal Fog").unwrap().values_at(x, y);
        assert_eq!(fog[1], 10.0);
        assert!((fog[5] - 0.4).abs() < 1e-6);
        assert_eq!(maps.get("Fog Blend Distance").unwrap().values_at(0.0, 0.0), vec![50.0]);
    }
}
