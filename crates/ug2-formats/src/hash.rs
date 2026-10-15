/// EA `bStringHash`: every name in the game (solids, textures, cars) is referenced by this.
pub fn bin_hash(s: &str) -> u32 {
    s.bytes().fold(0xFFFF_FFFF, |h: u32, c| h.wrapping_mul(33).wrapping_add(c as u32))
}

#[cfg(test)]
mod tests {
    #[test]
    fn known_solid_name() {
        // Hash stored next to the name in CARS/350Z/GEOMETRY.BIN.
        assert_eq!(super::bin_hash("350Z_BASE_A"), 0x4CC6_C50B);
    }
}
