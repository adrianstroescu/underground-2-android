//! Light materials (GLOBALB chunks 0x135200): how the car effect shades each surface
//! (paints, chrome, rubber, glass...). Car meshes name theirs through the solid's
//! shader hash (`CARSKIN` is the body paint, replaced by the chosen paint).
//!
//! 160 bytes: header, u32 name hash at 0x0C, name at 0x14, then 28 floats from 0x30:
//! - `[0]` diffuse min scale, `[1..3]` diffuse min RGB, `[4]` max scale, `[5..7]` max RGB,
//!   `[8]` diffuse min alpha, `[9]` max alpha;
//! - `[10]` specular power, `[11]` min scale, `[12..14]` min RGB, `[15]` max scale,
//!   `[16..18]` max RGB;
//! - `[19]` envmap power, `[20]` min scale, `[21..23]` min RGB, `[24]` max scale,
//!   `[25..27]` max RGB.
//!
//! Checked against the PC game's shader constants (apitrace): the effect gets
//! Min = scale x RGB, Range = Max - Min, and the specular power x 1.1.

use crate::bytes::{cstr_at, f32_at};
use crate::chunk::ChunkIter;

pub const LIGHT_MATERIAL: u32 = 0x0013_5200;

#[derive(Clone, Debug)]
pub struct LightMaterial {
    pub name: String,
    pub hash: u32,
    pub diffuse_min: [f32; 4],
    pub diffuse_range: [f32; 4],
    pub specular_min: [f32; 3],
    pub specular_range: [f32; 3],
    pub specular_power: f32,
    pub envmap_min: [f32; 3],
    pub envmap_range: [f32; 3],
    pub envmap_power: f32,
}

pub fn parse_light_materials(global_b: &[u8]) -> Vec<LightMaterial> {
    ChunkIter::new(global_b)
        .filter(|c| c.id == LIGHT_MATERIAL && c.data.len() >= 0x30 + 28 * 4)
        .filter_map(|c| {
            let p = c.data;
            let name = cstr_at(p, 0x14, 32).ok()?;
            let f = |i: usize| f32_at(p, 0x30 + i * 4).unwrap_or(0.0);
            let scaled = |s: usize| [f(s) * f(s + 1), f(s) * f(s + 2), f(s) * f(s + 3)];
            let range = |a: [f32; 3], b: [f32; 3]| [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let (dmin, dmax) = (scaled(0), scaled(4));
            let (smin, smax) = (scaled(11), scaled(15));
            let (emin, emax) = (scaled(20), scaled(24));
            Some(LightMaterial {
                hash: crate::bin_hash(&name),
                name,
                diffuse_min: [dmin[0], dmin[1], dmin[2], f(8)],
                diffuse_range: [dmax[0] - dmin[0], dmax[1] - dmin[1], dmax[2] - dmin[2], f(9) - f(8)],
                specular_min: smin,
                specular_range: range(smin, smax),
                specular_power: f(10) * 1.1,
                envmap_min: emin,
                envmap_range: range(emin, emax),
                envmap_power: f(19),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The values the PC game sent for a blue regular paint (apitrace capture).
    #[test]
    fn regpaintblue_matches_the_game() {
        let game = crate::fs::GameDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        let Ok(gb) = game.read("GLOBAL/GLOBALB.BUN") else { return };
        let all = parse_light_materials(&gb);
        assert_eq!(all.len(), 186);
        let m = all.iter().find(|m| m.name == "REGPAINTBLUE").unwrap();
        let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
        assert!(close(m.diffuse_min[0], 0.662) && close(m.diffuse_min[3], 0.993));
        assert!(close(m.diffuse_range[3], -0.828));
        assert!(close(m.specular_min[2], 1.038) && close(m.specular_range[2], -0.4584));
        assert!(close(m.specular_power, 5.72) && close(m.envmap_power, 0.262));
    }
}
