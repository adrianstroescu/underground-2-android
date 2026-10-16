//! World lights (street lamps, signs): container 0x80135000 in each streaming section.
//! 0x135003 holds 96-byte records after 0x11 alignment padding; at night the game draws
//! them as camera-facing glow sprites.

use crate::bytes::{f32_at, u32_at, vec3_at};
use crate::chunk::Chunk;
use crate::Result;

pub const LIGHT_PACK: u32 = 0x8013_5000;
const LIGHTS: u32 = 0x0013_5003;
const RECORD_LEN: usize = 96;

#[derive(Clone, Copy, Debug)]
pub struct WorldLight {
    pub hash: u32,
    /// Raw type bytes at +4.
    pub kind: u32,
    /// RGBA.
    pub color: [u8; 4],
    pub position: [f32; 3],
    /// Glow size, metres.
    pub size: f32,
    /// Direction the lamp faces (usually straight down).
    pub direction: [f32; 3],
}

pub fn parse_light_pack(pack: &Chunk) -> Result<Vec<WorldLight>> {
    let mut out = Vec::new();
    let Some(c) = pack.child(LIGHTS) else { return Ok(out) };
    // Records are aligned to the end of the chunk; the start holds 0x11 padding.
    let data = &c.data[c.data.len() % RECORD_LEN..];
    for rec in data.chunks_exact(RECORD_LEN) {
        let argb = u32_at(rec, 12)?;
        out.push(WorldLight {
            hash: u32_at(rec, 0)?,
            kind: u32_at(rec, 4)?,
            color: [(argb >> 16) as u8, (argb >> 8) as u8, argb as u8, (argb >> 24) as u8],
            position: vec3_at(rec, 16)?,
            size: f32_at(rec, 28)?,
            direction: vec3_at(rec, 32)?,
        });
    }
    Ok(out)
}
