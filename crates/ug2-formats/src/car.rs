//! Car part naming. Solid names are stored truncated to 27 characters, so parts are
//! looked up the way the game does it: build the full name and compare hashes.

use crate::bin_hash;

/// Body slots that make up a stock car (`<CAR>_KIT00_<SLOT>_A`).
const KIT_SLOTS: &[&str] = &[
    "BODY",
    "BRAKELIGHT",
    "DOOR_LEFT",
    "DOOR_RIGHT",
    "DOOR_PANEL_LEFT",
    "DOOR_PANEL_RIGHT",
    "DOOR_SILL_LEFT",
    "DOOR_SILL_RIGHT",
    "EXHAUST",
    "FRONT_BUMPER",
    "HEADLIGHT_LEFT",
    "HEADLIGHT_RIGHT",
    "HOOD",
    "HOOD_UNDER",
    "LEFT_SIDE_MIRROR",
    "RIGHT_SIDE_MIRROR",
    "REAR_BUMPER",
    "ROOF",
    "SKIRT",
    "TRUNK",
    "TRUNK_UNDER",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartKind {
    Body,
    FrontWheel,
    FrontBrake,
    RearBrake,
}

/// Full names (LOD A) of the parts of a stock car, with their role.
pub fn stock_parts(car: &str) -> Vec<(String, PartKind)> {
    let mut v = vec![(format!("{car}_BASE_A"), PartKind::Body)];
    v.extend(KIT_SLOTS.iter().map(|s| (format!("{car}_KIT00_{s}_A"), PartKind::Body)));
    v.push((format!("{car}_KIT00_FRONT_WHEEL_A"), PartKind::FrontWheel));
    v.push((format!("{car}_KIT00_FRONT_BRAKE_A"), PartKind::FrontBrake));
    v.push((format!("{car}_KIT00_REAR_BRAKE_A"), PartKind::RearBrake));
    v
}

/// (hash, kind) pairs for [`stock_parts`].
pub fn stock_part_hashes(car: &str) -> Vec<(u32, PartKind)> {
    stock_parts(car).into_iter().map(|(n, k)| (bin_hash(&n), k)).collect()
}

/// Parts of a traffic car (`<CAR>_BODY_A` and one tyre model for all four wheels).
pub fn traffic_parts(car: &str) -> Vec<(String, PartKind)> {
    vec![(format!("{car}_BODY_A"), PartKind::Body), (format!("{car}_TIRE_FRONT_A"), PartKind::FrontWheel)]
}

/// What a material the game fills in at runtime stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Material {
    /// The player's paint (and vinyls): `<CAR>_SKIN1` and the body-colour swatches.
    Paint,
    Window,
    Headlight,
    HeadlightGlass,
    Brakelight,
    BrakelightGlass,
}

/// Texture names that car parts reference but no texture pack contains: the game swaps
/// them for the car's own light textures, the shared window texture or the paint skin.
/// Returns `(referenced hash, replacement texture hash, material)`; the replacement is
/// `None` for paint. Traffic skins (`<CAR>_SKIN1` in CARS/TEXTURES.BIN) are real
/// textures, so callers should only treat the skin as paint when it is missing.
pub fn runtime_materials(car: &str) -> Vec<(u32, Option<u32>, Material)> {
    let h = |s: &str| bin_hash(s);
    let mut v = Vec::new();
    for w in ["WINDOW_FRONT", "WINDOW_REAR", "WINDOW_LEFT_FRONT", "WINDOW_LEFT_REAR", "WINDOW_RIGHT_FRONT", "WINDOW_RIGHT_REAR"] {
        v.push((h(w), Some(h("WINDOW")), Material::Window));
    }
    let light = |v: &mut Vec<_>, generic: &str, tex: String, m: Material| v.push((h(generic), Some(h(&tex)), m));
    for side in ["LEFT", "RIGHT"] {
        light(&mut v, &format!("HEADLIGHT_{side}"), format!("{car}_KIT00_HEADLIGHT_OFF"), Material::Headlight);
        light(&mut v, &format!("HEADLIGHT_GLASS_{side}"), format!("{car}_KIT00_HEADLIGHT_GLASS_OFF"), Material::HeadlightGlass);
        light(&mut v, &format!("BRAKELIGHT_{side}"), format!("{car}_KIT00_BRAKELIGHT_OFF"), Material::Brakelight);
        light(&mut v, &format!("BRAKELIGHT_GLASS_{side}"), format!("{car}_KIT00_BRAKELIGHT_GLASS_OFF"), Material::BrakelightGlass);
    }
    light(&mut v, "BRAKELIGHT_CENTRE", format!("{car}_CENTRE_BRAKELIGHT_OFF"), Material::Brakelight);
    v.push((h(&format!("{car}_SKIN1")), None, Material::Paint));
    v.push((h("CARSKIN_CARBONFIBRE"), None, Material::Paint));
    // Body-colour swatch on every panel; its name is unknown but the role is clear.
    v.push((0x3C84_D757, None, Material::Paint));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_material_hashes_match_the_game() {
        let m = runtime_materials("350Z");
        let find = |h: u32| m.iter().find(|e| e.0 == h).map(|e| e.2);
        // Hashes referenced by 350Z_KIT00_* parts.
        assert_eq!(find(0x7B22_0DDF), Some(Material::Window));
        assert_eq!(find(0xA7E6_EA53), Some(Material::Headlight));
        assert_eq!(find(0x000B_CBBF), Some(Material::BrakelightGlass));
        assert_eq!(find(0x3617_DCFC), Some(Material::Brakelight));
        assert_eq!(find(0xA675_4656), Some(Material::Paint));
        assert_eq!(bin_hash("350Z_KIT00_HEADLIGHT_OFF"), 0xF7C9_D5DB);
    }
}
