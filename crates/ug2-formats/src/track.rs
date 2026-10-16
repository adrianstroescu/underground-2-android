//! Track streaming: `TRACKS/L4RA.BUN` holds the section table, `TRACKS/STREAML4RA.BUN`
//! the section payloads (solids, textures, scenery placement).

use crate::bytes::{cstr_at, u32_at};
use crate::chunk::{ids, ChunkIter};
use crate::light::{parse_light_pack, WorldLight, LIGHT_PACK};
use crate::scenery::{parse_scenery_section, ScenerySection};
use crate::solid::{parse_solid_list, SolidList};
use crate::tpk::{parse_tpk, TexturePack};
use crate::{bail, Result};

const SECTION_ENTRY_LEN: usize = 0x50;

#[derive(Clone, Debug)]
pub struct StreamingSection {
    pub name: String,
    pub number: u32,
    pub offset: usize,
    pub size: usize,
}

impl StreamingSection {
    /// Sections numbered N*100 (e.g. "X0", "Y0") are shared / always resident.
    pub fn is_shared(&self) -> bool {
        self.number % 100 == 0
    }
}

pub fn parse_streaming_sections(index: &[u8]) -> Result<Vec<StreamingSection>> {
    let Some(table) = ChunkIter::new(index).find(|c| c.id == ids::TRACK_STREAMING_SECTIONS) else {
        bail!("no streaming section table");
    };
    let mut out = Vec::new();
    for e in table.data.chunks_exact(SECTION_ENTRY_LEN) {
        let size = u32_at(e, 0x18)? as usize;
        if size == 0 {
            continue;
        }
        out.push(StreamingSection {
            name: cstr_at(e, 0, 8)?,
            number: u32_at(e, 0x08)?,
            offset: u32_at(e, 0x14)? as usize,
            size,
        });
    }
    Ok(out)
}

#[derive(Default)]
pub struct SectionData {
    pub solids: Vec<SolidList>,
    pub textures: Vec<TexturePack>,
    pub scenery: Vec<ScenerySection>,
    pub lights: Vec<WorldLight>,
    pub collision: Vec<crate::collision::CollisionSection>,
}

/// Parses every chunk in `stream[start..start+len]`. Offsets stay absolute so that
/// alignment-dependent data (texture blobs) resolves correctly.
pub fn parse_region(stream: &[u8], start: usize, len: usize) -> SectionData {
    let mut out = SectionData::default();
    let end = (start + len).min(stream.len());
    let Some(region) = stream.get(start..end) else { return out };
    for c in ChunkIter::with_base(region, start) {
        match c.id {
            ids::SOLID_LIST => match parse_solid_list(&c) {
                Ok(l) => out.solids.push(l),
                Err(e) => log::warn!("solids at {:#x}: {e}", c.offset),
            },
            ids::TPK => match parse_tpk(stream, &c) {
                Ok(t) => out.textures.push(t),
                Err(e) => log::warn!("tpk at {:#x}: {e}", c.offset),
            },
            ids::SCENERY_SECTION => match parse_scenery_section(&c) {
                Ok(s) => out.scenery.push(s),
                Err(e) => log::warn!("scenery at {:#x}: {e}", c.offset),
            },
            crate::collision::TRACK_COLLISION => out.collision.extend(crate::collision::parse_collision(&c)),
            LIGHT_PACK => match parse_light_pack(&c) {
                Ok(l) => out.lights.extend(l),
                Err(e) => log::warn!("lights at {:#x}: {e}", c.offset),
            },
            _ => {}
        }
    }
    out
}
