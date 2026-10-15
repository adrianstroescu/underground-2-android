//! EA bChunk tree walking.

/// Chunk ids seen in UG2 files.
pub mod ids {
    pub const PADDING: u32 = 0x0000_0000;

    // Solid (geometry) lists
    pub const SOLID_LIST: u32 = 0x8013_4000;
    pub const SOLID_LIST_HEADER: u32 = 0x8013_4001;
    pub const SOLID_LIST_INFO: u32 = 0x0013_4002;
    pub const SOLID_OBJECT: u32 = 0x8013_4010;
    pub const SOLID_OBJECT_HEADER: u32 = 0x0013_4011;
    pub const SOLID_TEXTURE_REFS: u32 = 0x0013_4012;
    pub const SOLID_SHADER_REFS: u32 = 0x0013_4013;
    pub const SOLID_MESH: u32 = 0x8013_4100;
    pub const SOLID_MESH_HEADER: u32 = 0x0013_4900;
    pub const SOLID_MESH_VERTICES: u32 = 0x0013_4B01;
    pub const SOLID_MESH_GROUPS: u32 = 0x0013_4B02;
    pub const SOLID_MESH_INDICES: u32 = 0x0013_4B03;

    // Texture packs
    pub const TPK: u32 = 0xB330_0000;
    pub const TPK_HEADER: u32 = 0xB331_0000;
    pub const TPK_INFO: u32 = 0x3331_0001;
    pub const TPK_HASHES: u32 = 0x3331_0002;
    pub const TPK_COMPRESSED_BLOCKS: u32 = 0x3331_0003;
    pub const TPK_TEXTURE_INFOS: u32 = 0x3331_0004;
    pub const TPK_FORMATS: u32 = 0x3331_0005;
    pub const TPK_DATA: u32 = 0xB332_0000;
    pub const TPK_DATA_BLOB: u32 = 0x3332_0002;

    // Track
    pub const TRACK_STREAMING_SECTIONS: u32 = 0x0003_4110;
    pub const SCENERY_SECTION: u32 = 0x8003_4100;
    pub const SCENERY_SECTION_HEADER: u32 = 0x0003_4101;
    pub const SCENERY_INFOS: u32 = 0x0003_4102;
    pub const SCENERY_INSTANCES: u32 = 0x0003_4103;
}

#[derive(Clone, Copy, Debug)]
pub struct Chunk<'a> {
    pub id: u32,
    /// Chunk body (without the 8-byte header).
    pub data: &'a [u8],
    /// Absolute offset of the chunk header inside the buffer it was read from.
    pub offset: usize,
}

impl<'a> Chunk<'a> {
    pub fn is_container(&self) -> bool {
        self.id & 0x8000_0000 != 0
    }

    pub fn body_offset(&self) -> usize {
        self.offset + 8
    }

    pub fn children(&self) -> ChunkIter<'a> {
        ChunkIter::with_base(self.data, self.body_offset())
    }

    pub fn child(&self, id: u32) -> Option<Chunk<'a>> {
        self.children().find(|c| c.id == id)
    }

    /// Body with the leading 0x11 alignment padding removed.
    pub fn payload(&self) -> &'a [u8] {
        crate::bytes::strip_pad(self.data)
    }
}

pub struct ChunkIter<'a> {
    buf: &'a [u8],
    pos: usize,
    base: usize,
}

impl<'a> ChunkIter<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self::with_base(buf, 0)
    }

    pub fn with_base(buf: &'a [u8], base: usize) -> Self {
        Self { buf, pos: 0, base }
    }
}

impl<'a> Iterator for ChunkIter<'a> {
    type Item = Chunk<'a>;

    fn next(&mut self) -> Option<Chunk<'a>> {
        let hdr = self.buf.get(self.pos..self.pos + 8)?;
        let id = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
        let size = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as usize;
        let start = self.pos + 8;
        let Some(data) = self.buf.get(start..start + size) else {
            log::warn!("truncated chunk {id:08X} at {:#x}", self.base + self.pos);
            self.pos = self.buf.len();
            return None;
        };
        let chunk = Chunk { id, data, offset: self.base + self.pos };
        self.pos = start + size;
        Some(chunk)
    }
}

/// Depth-first visit of every chunk in `buf`.
pub fn walk<'a>(buf: &'a [u8], f: &mut impl FnMut(usize, &Chunk<'a>)) {
    fn rec<'a>(it: ChunkIter<'a>, depth: usize, f: &mut impl FnMut(usize, &Chunk<'a>)) {
        for c in it {
            f(depth, &c);
            if c.is_container() {
                rec(c.children(), depth + 1, f);
            }
        }
    }
    rec(ChunkIter::new(buf), 0, f);
}
