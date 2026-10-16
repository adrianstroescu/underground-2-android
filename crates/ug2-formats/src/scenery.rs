//! Scenery sections (chunk 0x80034100): which solids are placed where in the world.

use crate::bytes::{cstr_at, i16_at, u16_at, u32_at, vec3_at};
use crate::chunk::{ids, Chunk, ChunkIter};
use crate::Result;

const INFO_LEN: usize = 68;
const INSTANCE_LEN: usize = 64;

#[derive(Clone, Debug)]
pub struct SceneryInfo {
    pub name: String,
    /// Solid hashes for LOD A (highest), B and C. Zero when absent.
    pub lods: [u32; 3],
}

#[derive(Clone, Debug)]
pub struct SceneryInstance {
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
    /// Index into [`ScenerySection::infos`].
    pub info: u16,
    /// 0x10: shown only while a scenery group lists it (see [`SceneryGroups`]);
    /// 0x4 / 0x8: start/finish graphics (forward/backward); 0x200: player barrier.
    pub flags: u16,
    pub position: [f32; 3],
    /// Basis rows (D3D row-vector convention): world = pos + x*r0 + y*r1 + z*r2.
    pub rotation: [[f32; 3]; 3],
}

#[derive(Clone, Debug, Default)]
pub struct ScenerySection {
    pub number: u32,
    pub infos: Vec<SceneryInfo>,
    pub instances: Vec<SceneryInstance>,
}

impl SceneryInstance {
    /// Column-major 4x4 transform (as consumed by glam's `Mat4::from_cols_array`).
    pub fn matrix(&self) -> [f32; 16] {
        let r = &self.rotation;
        let p = &self.position;
        [
            r[0][0], r[0][1], r[0][2], 0.0, //
            r[1][0], r[1][1], r[1][2], 0.0, //
            r[2][0], r[2][1], r[2][2], 0.0, //
            p[0], p[1], p[2], 1.0,
        ]
    }
}

pub fn parse_scenery_section(c: &Chunk) -> Result<ScenerySection> {
    let mut s = ScenerySection::default();
    for sub in c.children() {
        match sub.id {
            ids::SCENERY_SECTION_HEADER => s.number = u32_at(sub.payload(), 0x0C)?,
            ids::SCENERY_INFOS => {
                for e in sub.payload().chunks_exact(INFO_LEN) {
                    s.infos.push(SceneryInfo {
                        name: cstr_at(e, 0, 32)?,
                        lods: [u32_at(e, 32)?, u32_at(e, 36)?, u32_at(e, 40)?],
                    });
                }
            }
            ids::SCENERY_INSTANCES => {
                for e in sub.payload().chunks_exact(INSTANCE_LEN) {
                    let mut rotation = [[0f32; 3]; 3];
                    for (i, v) in rotation.iter_mut().flatten().enumerate() {
                        *v = i16_at(e, 44 + i * 2)? as f32 / 8192.0;
                    }
                    s.instances.push(SceneryInstance {
                        bbox_min: vec3_at(e, 0)?,
                        bbox_max: vec3_at(e, 12)?,
                        info: u16_at(e, 24)?,
                        flags: u16_at(e, 26)?,
                        position: vec3_at(e, 32)?,
                        rotation,
                    });
                }
            }
            _ => {}
        }
    }
    s.instances.retain(|i| (i.info as usize) < s.infos.len());
    Ok(s)
}

/// True for top-level chunk ids that hold scenery.
pub fn is_scenery(id: u32) -> bool {
    id == ids::SCENERY_SECTION
}

/// Instance flag: shown only while one of the groups that list it is enabled.
pub const GROUP_CONTROLLED: u16 = 0x10;

const SCENERY_OVERRIDES: u32 = 0x0003_4107;
const SCENERY_GROUPS: u32 = 0x0003_4108;

/// Scenery groups of a region (chunks 0x34107 / 0x34108 of its index BUN): named sets
/// of instances the game switches on together, e.g. `BARRIERS_4301` (the barriers that
/// shape one race's track), `PLAYER_BARRIERS_4001` or `FREE_ROAM` (shop markers).
#[derive(Clone, Debug, Default)]
pub struct SceneryGroups {
    /// (scenery section number, instance index) per override entry.
    pub overrides: Vec<(u16, u16)>,
    /// Group name and the override entries it switches on.
    pub groups: Vec<(String, Vec<u16>)>,
}

impl SceneryGroups {
    pub fn parse(index: &[u8]) -> Self {
        let mut out = Self::default();
        for c in ChunkIter::new(index) {
            let p = c.payload();
            match c.id {
                // 8 bytes: section, instance, flags to switch, number of groups using it.
                SCENERY_OVERRIDES => {
                    out.overrides = p
                        .chunks_exact(8)
                        .map(|e| (u16::from_le_bytes([e[0], e[1]]), u16::from_le_bytes([e[2], e[3]])))
                        .collect();
                }
                // Per group: 8 bytes, name[32], hash, 0, u32 count, count x u16 (4-aligned).
                SCENERY_GROUPS => {
                    let mut o = 0;
                    while o + 52 <= p.len() {
                        let name = cstr_at(p, o + 8, 32).unwrap_or_default();
                        let n = u32_at(p, o + 48).unwrap_or(0) as usize;
                        let end = o + 52 + n * 2;
                        if end > p.len() {
                            break;
                        }
                        let entries = (0..n).filter_map(|k| u16_at(p, o + 52 + k * 2).ok()).collect();
                        out.groups.push((name, entries));
                        o = o + 52 + ((n * 2 + 3) & !3);
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// For each (section, instance) under group control: the indices of the groups that
    /// list it.
    pub fn membership(&self) -> std::collections::HashMap<(u32, u16), Vec<u16>> {
        let mut m: std::collections::HashMap<(u32, u16), Vec<u16>> = std::collections::HashMap::new();
        for (gi, (_, entries)) in self.groups.iter().enumerate() {
            for &e in entries {
                if let Some(&(sec, inst)) = self.overrides.get(e as usize) {
                    let v = m.entry((sec as u32, inst)).or_default();
                    if !v.contains(&(gi as u16)) {
                        v.push(gi as u16);
                    }
                }
            }
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drift_lot_groups() {
        let game = crate::fs::GameDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        let Ok(index) = game.read("TRACKS/L4RC.BUN") else { return };
        let g = SceneryGroups::parse(&index);
        assert_eq!(g.overrides.len(), 1568);
        let names: Vec<&str> = g.groups.iter().map(|x| x.0.as_str()).collect();
        assert!(names.contains(&"BARRIERS_4301") && names.contains(&"BARRIERS_4315"), "{names:?}");
        let b4301 = g.groups.iter().find(|x| x.0 == "BARRIERS_4301").unwrap();
        assert_eq!(b4301.1.len(), 21);
        // The yellow barrier across the 4301 route belongs to the 4305/4306 layouts only.
        let m = g.membership();
        let groups: Vec<&str> = m[&(313, 115)].iter().map(|&i| g.groups[i as usize].0.as_str()).collect();
        assert!(groups.contains(&"BARRIERS_4305") && groups.contains(&"BARRIERS_4306") && groups.len() == 2, "{groups:?}");
    }
}
