//! Scenery sections (chunk 0x80034100): which solids are placed where in the world.

use crate::bytes::{cstr_at, i16_at, u16_at, u32_at, vec3_at};
use crate::chunk::{ids, Chunk};
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
