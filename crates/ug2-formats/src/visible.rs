//! Visible section manager (chunk 0x80034150): which scenery sections the game draws
//! while the player is on each drivable section.
//!
//! Regions such as the drag strips hold several venues side by side, each with its own
//! backdrop panoramas; drawing every section at once puts one venue's panorama walls
//! across another venue's road.

use crate::bytes::{f32_at, u16_at, u32_at};
use crate::chunk::ChunkIter;
use std::collections::HashMap;

const MANAGER: u32 = 0x8003_4150;
const DRIVABLE: u32 = 0x0003_4151;
const BOUNDARIES: u32 = 0x0003_4152;
const VISIBLE_LISTS: u32 = 0x0003_4153;
const LIST_LEN: usize = 164;

/// Outline of a section on the ground plane.
#[derive(Clone, Debug)]
pub struct Boundary {
    pub section: u16,
    pub bbox_min: [f32; 2],
    pub bbox_max: [f32; 2],
    pub points: Vec<[f32; 2]>,
}

impl Boundary {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        if x < self.bbox_min[0] || x > self.bbox_max[0] || y < self.bbox_min[1] || y > self.bbox_max[1] {
            return false;
        }
        let n = self.points.len();
        let mut inside = false;
        for i in 0..n {
            let [ax, ay] = self.points[i];
            let [bx, by] = self.points[(i + 1) % n];
            if (ay > y) != (by > y) && x < ax + (y - ay) * (bx - ax) / (by - ay) {
                inside = !inside;
            }
        }
        inside
    }

    /// Distance from the point to the outline's bounding box (0 inside it).
    fn bbox_distance(&self, x: f32, y: f32) -> f32 {
        let dx = (self.bbox_min[0] - x).max(x - self.bbox_max[0]).max(0.0);
        let dy = (self.bbox_min[1] - y).max(y - self.bbox_max[1]).max(0.0);
        dx.hypot(dy)
    }
}

#[derive(Clone, Debug, Default)]
pub struct VisibleSections {
    /// Drivable section numbers (0x34151).
    pub drivable: Vec<u16>,
    /// Outlines of drivable sections and of panoramas (0x34152).
    pub boundaries: Vec<Boundary>,
    /// Sections drawn from each drivable section (0x34153).
    pub lists: HashMap<u16, Vec<u16>>,
}

impl VisibleSections {
    /// Reads the manager from a region index (`TRACKS/L4Rx.BUN`); `None` without one.
    pub fn parse(index: &[u8]) -> Option<Self> {
        let manager = ChunkIter::new(index).find(|c| c.id == MANAGER)?;
        let mut out = Self::default();
        for c in manager.children() {
            let p = c.data;
            match c.id {
                // u32 version, u32 count, count x u16.
                DRIVABLE => {
                    let n = u32_at(p, 4).unwrap_or(0) as usize;
                    out.drivable = (0..n).filter_map(|k| u16_at(p, 8 + k * 2).ok()).collect();
                }
                // 8 bytes, u16 section, u16 points, bbox min/max, centre, points (x, y).
                BOUNDARIES => {
                    let mut o = 0;
                    while o + 36 <= p.len() {
                        let (Ok(section), Ok(n)) = (u16_at(p, o + 8), u16_at(p, o + 10)) else { break };
                        let n = n as usize;
                        let end = o + 36 + n * 8;
                        if end > p.len() {
                            break;
                        }
                        let f = |at: usize| f32_at(p, at).unwrap_or(0.0);
                        out.boundaries.push(Boundary {
                            section,
                            bbox_min: [f(o + 12), f(o + 16)],
                            bbox_max: [f(o + 20), f(o + 24)],
                            points: (0..n).map(|k| [f(o + 36 + k * 8), f(o + 40 + k * 8)]).collect(),
                        });
                        o = end;
                    }
                }
                // Fixed 164-byte records: 8 bytes, u16 section, 6 bytes, u32 count, u16 list.
                VISIBLE_LISTS => {
                    for r in p.chunks_exact(LIST_LEN) {
                        let (Ok(section), Ok(n)) = (u16_at(r, 8), u32_at(r, 16)) else { continue };
                        let n = (n as usize).min((LIST_LEN - 20) / 2);
                        out.lists.insert(section, (0..n).filter_map(|k| u16_at(r, 20 + k * 2).ok()).collect());
                    }
                }
                _ => {}
            }
        }
        (!out.lists.is_empty()).then_some(out)
    }

    /// The drivable section under a ground position. Off every outline (a car on a
    /// verge or in the air), the one whose outline is nearest.
    pub fn drivable_at(&self, x: f32, y: f32) -> Option<u16> {
        let mut best: Option<(f32, u16)> = None;
        for b in self.boundaries.iter().filter(|b| self.lists.contains_key(&b.section)) {
            if b.contains(x, y) {
                return Some(b.section);
            }
            let d = b.bbox_distance(x, y);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, b.section));
            }
        }
        best.map(|(_, s)| s)
    }

    /// Sections drawn while on `drivable`.
    pub fn visible_from(&self, drivable: u16) -> Option<&[u16]> {
        self.lists.get(&drivable).map(Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_venues_see_their_own_panoramas() {
        let game = crate::fs::GameDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        let Ok(index) = game.read("TRACKS/L4RB.BUN") else { return };
        let v = VisibleSections::parse(&index).unwrap();
        assert_eq!(v.drivable.len(), 15);
        assert_eq!(v.lists.len(), 15);
        // The city drag strip (race 4201) starts on the second venue.
        let s = v.drivable_at(986.0, 42.0).unwrap();
        assert_eq!(s / 100, 2);
        let list = v.visible_from(s).unwrap();
        assert!(list.contains(&224) && list.contains(&242));
        assert!(!list.contains(&141), "the airport panorama belongs to the first venue");
    }

    #[test]
    fn bayview_lists() {
        let game = crate::fs::GameDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        let Ok(index) = game.read("TRACKS/L4RA.BUN") else { return };
        let v = VisibleSections::parse(&index).unwrap();
        assert_eq!(v.lists.len(), 255);
        assert!(v.boundaries.len() > v.lists.len());
        assert_eq!(v.visible_from(101).unwrap().first(), Some(&101));
    }
}
