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

/// Visual parts fitted to a racer; 0 means stock everywhere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PartSet {
    /// Body kit 1..29: front and rear bumper and side skirts.
    pub kit: u8,
    /// Widebody 1..4: body, doors, door panels and sills.
    pub widebody: u8,
    /// Hood style 1..28 (21..25 come in left and right halves).
    pub hood: u8,
    /// Headlight and brake-light style 1..14.
    pub lights: u8,
    /// Rim style (1-based index into the rim catalogue), 0 = stock wheels.
    pub rim: u16,
    /// Spoiler style 1..40, 0 = stock.
    pub spoiler: u8,
    /// Carbon-fibre spoiler.
    pub spoiler_cf: bool,
}

impl PartSet {
    pub fn is_stock(&self) -> bool {
        *self == Self::default()
    }

    /// Short key used to tell loaded models apart, e.g. "K03W00H05L02".
    pub fn key(&self) -> String {
        format!(
            "K{:02}W{:02}H{:02}L{:02}R{:03}S{:02}{}",
            self.kit,
            self.widebody,
            self.hood,
            self.lights,
            self.rim,
            self.spoiler,
            if self.spoiler_cf { "C" } else { "" }
        )
    }
}

/// Parts of a racer with `parts` fitted: for each part, candidate full names (LOD A) in
/// order of preference; the first one the car has is used, so missing styles fall back
/// to stock.
pub fn fitted_parts(car: &str, parts: &PartSet) -> Vec<(Vec<String>, PartKind)> {
    let stock = |slot: &str| format!("{car}_KIT00_{slot}_A");
    let mut v = vec![(vec![format!("{car}_BASE_A")], PartKind::Body)];
    for slot in KIT_SLOTS {
        let mut c = Vec::new();
        match *slot {
            "FRONT_BUMPER" | "REAR_BUMPER" | "SKIRT" if parts.kit > 0 => {
                c.push(format!("{car}_KIT{:02}_{slot}_A", parts.kit));
            }
            "BODY" | "DOOR_LEFT" | "DOOR_RIGHT" | "DOOR_PANEL_LEFT" | "DOOR_PANEL_RIGHT" | "DOOR_SILL_LEFT"
            | "DOOR_SILL_RIGHT"
                if parts.widebody > 0 =>
            {
                c.push(format!("{car}_KITW{:02}_{slot}_A", parts.widebody));
            }
            "HEADLIGHT_LEFT" | "HEADLIGHT_RIGHT" | "BRAKELIGHT" if parts.lights > 0 => {
                c.push(format!("{car}_STYLE{:02}_{slot}_A", parts.lights));
            }
            "HOOD" if parts.hood > 0 => {
                if (21..=25).contains(&parts.hood) {
                    // Split hoods: the right half is its own part.
                    v.push((vec![format!("{car}_STYLE{:02}_HOOD_RIGHT_A", parts.hood)], PartKind::Body));
                    c.push(format!("{car}_STYLE{:02}_HOOD_LEFT_A", parts.hood));
                } else {
                    c.push(format!("{car}_STYLE{:02}_HOOD_A", parts.hood));
                }
            }
            _ => {}
        }
        c.push(stock(slot));
        v.push((c, PartKind::Body));
    }
    v.push((vec![stock("FRONT_WHEEL")], PartKind::FrontWheel));
    v.push((vec![stock("FRONT_BRAKE")], PartKind::FrontBrake));
    v.push((vec![stock("REAR_BRAKE")], PartKind::RearBrake));
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
/// Returns `(referenced hash, replacement candidates, material)`: the first candidate the
/// car has wins (light styles fall back to stock); no candidates means paint. Traffic
/// skins (`<CAR>_SKIN1` in CARS/TEXTURES.BIN) are real textures, so callers should only
/// treat the skin as paint when it is missing. `lights` is the light style (0 = stock).
pub fn runtime_materials(car: &str, lights: u8) -> Vec<(u32, Vec<u32>, Material)> {
    let h = |s: &str| bin_hash(s);
    let mut v = Vec::new();
    for w in ["WINDOW_FRONT", "WINDOW_REAR", "WINDOW_LEFT_FRONT", "WINDOW_LEFT_REAR", "WINDOW_RIGHT_FRONT", "WINDOW_RIGHT_REAR"] {
        v.push((h(w), vec![h("WINDOW")], Material::Window));
    }
    let style = if lights > 0 { format!("STYLE{lights:02}") } else { "KIT00".to_string() };
    let light = |v: &mut Vec<_>, generic: &str, part: &str, m: Material| {
        let tex = [format!("{car}_{style}_{part}"), format!("{car}_KIT00_{part}")];
        v.push((h(generic), tex.iter().map(|t| h(t)).collect(), m));
    };
    for side in ["LEFT", "RIGHT"] {
        light(&mut v, &format!("HEADLIGHT_{side}"), "HEADLIGHT_OFF", Material::Headlight);
        light(&mut v, &format!("HEADLIGHT_GLASS_{side}"), "HEADLIGHT_GLASS_OFF", Material::HeadlightGlass);
        light(&mut v, &format!("BRAKELIGHT_{side}"), "BRAKELIGHT_OFF", Material::Brakelight);
        light(&mut v, &format!("BRAKELIGHT_GLASS_{side}"), "BRAKELIGHT_GLASS_OFF", Material::BrakelightGlass);
    }
    v.push((h("BRAKELIGHT_CENTRE"), vec![h(&format!("{car}_CENTRE_BRAKELIGHT_OFF"))], Material::Brakelight));
    v.push((h(&format!("{car}_SKIN1")), Vec::new(), Material::Paint));
    v.push((h("CARSKIN_CARBONFIBRE"), Vec::new(), Material::Paint));
    // Body-colour swatch on every panel; its name is unknown but the role is clear.
    v.push((0x3C84_D757, Vec::new(), Material::Paint));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_material_hashes_match_the_game() {
        let m = runtime_materials("350Z", 0);
        let find = |h: u32| m.iter().find(|e| e.0 == h).map(|e| e.2);
        // Hashes referenced by 350Z_KIT00_* parts.
        assert_eq!(find(0x7B22_0DDF), Some(Material::Window));
        assert_eq!(find(0xA7E6_EA53), Some(Material::Headlight));
        assert_eq!(find(0x000B_CBBF), Some(Material::BrakelightGlass));
        assert_eq!(find(0x3617_DCFC), Some(Material::Brakelight));
        assert_eq!(find(0xA675_4656), Some(Material::Paint));
        assert_eq!(bin_hash("350Z_KIT00_HEADLIGHT_OFF"), 0xF7C9_D5DB);
    }

    #[test]
    fn fitted_parts_exist_in_the_game() {
        let game = crate::fs::GameDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        let Ok(geo) = game.read("CARS/350Z/GEOMETRY.BIN") else { return };
        let solids: Vec<u32> =
            crate::solid::find_solid_lists(&geo).into_iter().flat_map(|l| l.solids).map(|s| s.hash).collect();
        let parts = PartSet { kit: 3, widebody: 1, hood: 22, lights: 5, ..Default::default() };
        for (names, _) in fitted_parts("350Z", &parts) {
            // The preferred (first) candidate should exist for every fitted style.
            assert!(solids.contains(&bin_hash(&names[0])), "missing {}", names[0]);
        }
    }
}
