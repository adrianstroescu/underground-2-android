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
