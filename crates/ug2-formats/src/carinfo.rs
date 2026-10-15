//! Car type table (`CarTypeInfo`, chunk 0x00034600 in GLOBAL/GLOBALB): one 0x890-byte
//! record per car with names, file paths, wheel placement and handling data.

use crate::bytes::{cstr_at, f32_at, vec3_at};
use crate::chunk::ChunkIter;
use crate::{bail, Result};

const CAR_TYPE_INFO: u32 = 0x0003_4600;
const RECORD_LEN: usize = 0x890;
const WHEELS_AT: usize = 0x120;
const WHEEL_LEN: usize = 0x30;

#[derive(Clone, Copy, Debug, Default)]
pub struct WheelInfo {
    /// Hub position in car space (X forward, Y left, Z up).
    pub position: [f32; 3],
    pub radius: f32,
    pub width: f32,
}

#[derive(Clone, Debug)]
pub struct CarTypeInfo {
    pub name: String,
    pub wheels: [WheelInfo; 4],
}

pub fn parse_car_types(global_b: &[u8]) -> Result<Vec<CarTypeInfo>> {
    let Some(chunk) = ChunkIter::new(global_b).find(|c| c.id == CAR_TYPE_INFO) else {
        bail!("no CarTypeInfo chunk");
    };
    // Records are aligned to the end of the chunk; the start holds alignment padding.
    let data = chunk.data;
    let start = data.len() % RECORD_LEN;
    let mut out = Vec::new();
    for rec in data[start..].chunks_exact(RECORD_LEN) {
        let mut wheels = [WheelInfo::default(); 4];
        for (i, w) in wheels.iter_mut().enumerate() {
            let o = WHEELS_AT + i * WHEEL_LEN;
            *w = WheelInfo { position: vec3_at(rec, o)?, radius: f32_at(rec, o + 0x10)?, width: f32_at(rec, o + 0x14)? };
        }
        out.push(CarTypeInfo { name: cstr_at(rec, 0, 32)?, wheels });
    }
    Ok(out)
}
