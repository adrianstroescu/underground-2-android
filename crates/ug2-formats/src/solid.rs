//! Solid lists (chunk 0x80134000): the meshes used for cars, track scenery and props.

use crate::bytes::{cstr_at, f32_at, mat4_at, u32_at, u8_at, vec3_at};
use crate::chunk::{ids, Chunk, ChunkIter};
use crate::{bail, Result};

/// Unified vertex. Track geometry has no normals; those get `[0, 0, 1]`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    /// D3DCOLOR (0xAARRGGBB), baked lighting on track geometry.
    pub color: u32,
    pub uv: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct MeshGroup {
    pub first_index: u32,
    pub index_count: u32,
    /// Index into [`Solid::textures`].
    pub texture: u8,
    /// Index into [`Solid::shaders`].
    pub shader: u8,
    pub flags: u32,
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
}

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    /// Triangle list.
    pub indices: Vec<u16>,
    pub groups: Vec<MeshGroup>,
    pub vertex_stride: usize,
    pub has_normals: bool,
}

#[derive(Clone, Debug)]
pub struct Solid {
    pub hash: u32,
    pub name: String,
    /// Row-major 4x4, row vectors (D3D convention): translation in elements 12..15.
    pub matrix: [f32; 16],
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
    pub textures: Vec<u32>,
    pub shaders: Vec<u32>,
    pub mesh: Option<Mesh>,
}

#[derive(Clone, Debug, Default)]
pub struct SolidList {
    pub path: String,
    pub section: String,
    pub solids: Vec<Solid>,
}

const GROUP_LEN: usize = 60;

fn hash_list(c: Option<Chunk>) -> Vec<u32> {
    c.map(|c| c.payload().chunks_exact(8).map(|e| u32::from_le_bytes(e[0..4].try_into().unwrap())).collect())
        .unwrap_or_default()
}

fn parse_vertex(v: &[u8], stride: usize) -> Result<Vertex> {
    let pos = vec3_at(v, 0)?;
    Ok(match stride {
        36 => Vertex { pos, normal: vec3_at(v, 12)?, color: u32_at(v, 24)?, uv: [f32_at(v, 28)?, f32_at(v, 32)?] },
        24 => Vertex { pos, normal: [0.0, 0.0, 1.0], color: u32_at(v, 12)?, uv: [f32_at(v, 16)?, f32_at(v, 20)?] },
        // Second UV set (unused for now).
        32 => Vertex { pos, normal: [0.0, 0.0, 1.0], color: u32_at(v, 12)?, uv: [f32_at(v, 16)?, f32_at(v, 20)?] },
        _ => bail!("unsupported vertex stride {stride}"),
    })
}

fn parse_mesh(mesh: &Chunk) -> Result<Mesh> {
    let Some(header) = mesh.child(ids::SOLID_MESH_HEADER) else { bail!("mesh without header") };
    let header = header.payload();
    let num_groups = u32_at(header, 0x10)? as usize;
    let num_tris = u32_at(header, 0x24)? as usize;
    let num_verts = u32_at(header, 0x34)? as usize;

    let vbuf = mesh.child(ids::SOLID_MESH_VERTICES).map(|c| c.payload()).unwrap_or(&[]);
    let gbuf = mesh.child(ids::SOLID_MESH_GROUPS).map(|c| c.payload()).unwrap_or(&[]);
    let ibuf = mesh.child(ids::SOLID_MESH_INDICES).map(|c| c.payload()).unwrap_or(&[]);

    if num_verts == 0 {
        return Ok(Mesh::default());
    }
    let stride = vbuf.len() / num_verts;
    if vbuf.len() - stride * num_verts >= 16 {
        bail!("vertex buffer {} bytes does not fit {num_verts} vertices", vbuf.len());
    }
    let vertices = vbuf
        .chunks_exact(stride)
        .take(num_verts)
        .map(|v| parse_vertex(v, stride))
        .collect::<Result<Vec<_>>>()?;

    let num_indices = num_tris * 3;
    if ibuf.len() < num_indices * 2 {
        bail!("index buffer too small: {} < {}", ibuf.len(), num_indices * 2);
    }
    let indices: Vec<u16> =
        ibuf.chunks_exact(2).take(num_indices).map(|i| u16::from_le_bytes([i[0], i[1]])).collect();
    if let Some(&bad) = indices.iter().find(|&&i| i as usize >= num_verts) {
        bail!("index {bad} out of range ({num_verts} vertices)");
    }

    let mut groups = Vec::with_capacity(num_groups);
    for g in gbuf.chunks_exact(GROUP_LEN).take(num_groups) {
        groups.push(MeshGroup {
            bbox_min: vec3_at(g, 0)?,
            index_count: u32_at(g, 12)?,
            bbox_max: vec3_at(g, 16)?,
            texture: u8_at(g, 28)?,
            shader: u8_at(g, 32)?,
            first_index: u32_at(g, 52)?,
            flags: u32_at(g, 56)?,
        });
    }
    groups.retain(|g| (g.first_index + g.index_count) as usize <= indices.len());

    Ok(Mesh { has_normals: stride == 36, vertex_stride: stride, vertices, indices, groups })
}

fn parse_object(obj: &Chunk) -> Result<Solid> {
    let Some(h) = obj.child(ids::SOLID_OBJECT_HEADER) else { bail!("solid without header") };
    let h = h.payload();
    let mesh = match obj.child(ids::SOLID_MESH) {
        Some(m) => Some(parse_mesh(&m)?),
        None => None,
    };
    Ok(Solid {
        hash: u32_at(h, 0x10)?,
        bbox_min: vec3_at(h, 0x20)?,
        bbox_max: vec3_at(h, 0x30)?,
        matrix: mat4_at(h, 0x40)?,
        name: cstr_at(h, 0xA4, 28)?,
        textures: hash_list(obj.child(ids::SOLID_TEXTURE_REFS)),
        shaders: hash_list(obj.child(ids::SOLID_SHADER_REFS)),
        mesh,
    })
}

pub fn parse_solid_list(list: &Chunk) -> Result<SolidList> {
    let mut out = SolidList::default();
    if let Some(info) = list.child(ids::SOLID_LIST_HEADER).and_then(|h| h.child(ids::SOLID_LIST_INFO)) {
        out.path = cstr_at(info.data, 0x10, 56)?;
        out.section = cstr_at(info.data, 0x48, 32)?;
    }
    for obj in list.children().filter(|c| c.id == ids::SOLID_OBJECT) {
        match parse_object(&obj) {
            Ok(s) => out.solids.push(s),
            Err(e) => log::warn!("solid at {:#x} in {}: {e}", obj.offset, out.path),
        }
    }
    Ok(out)
}

/// Every top-level solid list in `buf`.
pub fn find_solid_lists(buf: &[u8]) -> Vec<SolidList> {
    ChunkIter::new(buf)
        .filter(|c| c.id == ids::SOLID_LIST)
        .filter_map(|c| match parse_solid_list(&c) {
            Ok(l) => Some(l),
            Err(e) => {
                log::warn!("solid list at {:#x}: {e}", c.offset);
                None
            }
        })
        .collect()
}
