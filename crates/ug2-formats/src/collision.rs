//! Track collision (chunk 0x80034130, one per drivable section in the stream file):
//! the polygons the game's physics drives on and bumps into, separate from the render
//! meshes. Road, kerbs, ramps and terrain are near-flat polygons; barriers and the
//! invisible map edges are vertical ones.
//!
//! 0x34131 header: u32 scenery section, u16 grid columns, u16 grid rows, grid origin
//! (x, y), bounds min and max (x, y, z, w).
//! 0x34134 polygons, 32 bytes each:
//!   u8 barrier, u8 shape, u8 surface, u8 flags, i16 base height (m), u16 0xFFFF,
//!   i16 x[4] (1/8 m), i16 y[4] (1/8 m), i16 z[4] (1/256 m above the base).
//!   A triangle repeats its last corner.
//! 0x34136 / 0x34137: 32 m grid cells listing polygons (not needed to rebuild them).
//!
//! Byte 0 of a polygon is 0 for permanent collision, or an index into the region's
//! barrier table (chunk 0x3412F in the index): walls that exist only while the race
//! named there runs (`BARRIERS_4221`, `PLAYER_BARRIERS_4105`...).

use crate::bytes::{cstr_at, i16_at, u32_at};
use crate::chunk::{Chunk, ChunkIter};

pub const TRACK_COLLISION: u32 = 0x8003_4130;
const HEADER: u32 = 0x0003_4131;
const POLYGONS: u32 = 0x0003_4134;
const POLYGON_LEN: usize = 32;
const BARRIER_NAMES: u32 = 0x0003_412F;

#[derive(Clone, Debug)]
pub struct CollisionPolygon {
    pub corners: [[f32; 3]; 4],
    /// Surface kind (asphalt, concrete barrier, grass...), byte 2.
    pub surface: u8,
    /// Byte 3; 0x02 is set on barriers and map-edge walls.
    pub flags: u8,
    /// Race barrier index (see [`barrier_names`]); 0 = always there.
    pub barrier: u8,
}

impl CollisionPolygon {
    /// One or two triangles (fan from corner 0), without degenerate ones.
    pub fn triangles(&self) -> impl Iterator<Item = [[f32; 3]; 3]> + '_ {
        let c = &self.corners;
        [[c[0], c[1], c[2]], [c[0], c[2], c[3]]].into_iter().filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
    }
}

#[derive(Clone, Debug, Default)]
pub struct CollisionSection {
    /// Scenery section number the polygons belong to (for visibility).
    pub section: u32,
    pub polygons: Vec<CollisionPolygon>,
}

pub fn parse_collision(c: &Chunk) -> Option<CollisionSection> {
    let mut out = CollisionSection::default();
    for part in c.children() {
        let p = part.data;
        match part.id {
            HEADER => out.section = u32_at(p, 0).ok()?,
            POLYGONS => {
                out.polygons = p
                    .chunks_exact(POLYGON_LEN)
                    .filter_map(|r| {
                        let base = i16_at(r, 4).ok()? as f32;
                        let v = |k: usize| i16_at(r, 8 + k * 2).map(|v| v as f32).unwrap_or(0.0);
                        let corners = std::array::from_fn(|k| [v(k) / 8.0, v(4 + k) / 8.0, base + v(8 + k) / 256.0]);
                        Some(CollisionPolygon { corners, surface: r[2], flags: r[3], barrier: r[0] })
                    })
                    .collect();
            }
            _ => {}
        }
    }
    Some(out)
}

/// Names of the race barrier sets, by the index polygons store (index 0 is "NONE").
/// 32-byte records: u32 name hash, u32 polygon count, u8 1, name.
pub fn barrier_names(index: &[u8]) -> Vec<String> {
    let Some(c) = ChunkIter::new(index).find(|c| c.id == BARRIER_NAMES) else { return Vec::new() };
    c.data.chunks_exact(32).map(|r| cstr_at(r, 9, 23).unwrap_or_default()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The city drag strip's start: the polygons sit at the road height of the race
    /// route there (2.25 m), not at the raw values.
    #[test]
    fn drag_start_height() {
        let game = crate::fs::GameDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        let Ok(stream) = game.read("TRACKS/STREAML4RB.BUN") else { return };
        let sections: Vec<CollisionSection> =
            ChunkIter::new(&stream).filter(|c| c.id == TRACK_COLLISION).filter_map(|c| parse_collision(&c)).collect();
        assert_eq!(sections.len(), 15);
        let s = sections.iter().find(|s| s.section == 221).unwrap();
        let near: Vec<f32> = s
            .polygons
            .iter()
            .filter(|p| p.corners.iter().all(|c| (c[0] - 984.0).abs() < 12.0 && (c[1] - 44.0).abs() < 12.0))
            .map(|p| p.corners[0][2])
            .collect();
        assert!(!near.is_empty());
        assert!(near.iter().all(|z| (z - 2.2).abs() < 0.6), "{near:?}");
    }

    #[test]
    fn drift_barrier_table() {
        let game = crate::fs::GameDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        let Ok(index) = game.read("TRACKS/L4RC.BUN") else { return };
        let names = barrier_names(&index);
        assert_eq!(names.len(), 12);
        assert_eq!(names[0], "NONE");
        assert_eq!(names[1], "BARRIERS_4311");
        assert_eq!(names[6], "BARRIERS_4301");
    }
}
