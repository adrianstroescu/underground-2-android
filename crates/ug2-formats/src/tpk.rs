//! Texture packs (`TPK`, chunk 0xB3300000).
//!
//! Two layouts exist:
//! * raw: per-texture info records (0x33310004) + format records (0x33310005) and
//!   one data blob (0x33320002) addressed by each record's data offset;
//! * compressed (cars, front end): a block table (0x33310003) pointing at JDLZ/HUFF
//!   blocks, each of which decompresses to `pixels | TextureInfo | FormatInfo`.

use crate::bytes::{cstr_at, slice, u16_at, u32_at, u8_at};
use crate::chunk::{ids, Chunk, ChunkIter};
use crate::compression;
use crate::dxt::{self, Bc};
use crate::{bail, Result};

const TEXTURE_INFO_LEN: usize = 0x7C;
const FORMAT_INFO_LEN: usize = 0x20;
const BLOCK_ENTRY_LEN: usize = 0x18;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TexFormat {
    Bc(Bc),
    /// D3DFMT_A8R8G8B8 / X8R8G8B8: bytes are B, G, R, A.
    Bgra8 { opaque: bool },
    /// 8-bit palettized, palette is BGRA.
    P8,
    Unknown(u32),
}

/// How the texture uses alpha (TextureInfo::AlphaUsageType).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlphaUsage {
    Opaque,
    Cutout,
    Blend,
}

#[derive(Clone, Debug)]
pub struct Texture {
    pub hash: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub mip_count: u32,
    pub format: TexFormat,
    /// All mip levels back to back (indices only for P8).
    pub data: Vec<u8>,
    /// BGRA palette for P8.
    pub palette: Vec<u8>,
    pub alpha: AlphaUsage,
    pub tileable: u8,
}

#[derive(Clone, Debug, Default)]
pub struct TexturePack {
    pub name: String,
    pub path: String,
    pub hash: u32,
    pub textures: Vec<Texture>,
}

struct TexInfo {
    name: String,
    hash: u32,
    data_offset: usize,
    palette_offset: usize,
    data_size: usize,
    palette_size: usize,
    width: u32,
    height: u32,
    compression: u8,
    mips: u8,
    tileable: u8,
    alpha_usage: u8,
}

fn parse_info(b: &[u8]) -> Result<TexInfo> {
    Ok(TexInfo {
        name: cstr_at(b, 0x0C, 24)?,
        hash: u32_at(b, 0x24)?,
        data_offset: u32_at(b, 0x30)? as usize,
        palette_offset: u32_at(b, 0x34)? as usize,
        data_size: u32_at(b, 0x38)? as usize,
        palette_size: u32_at(b, 0x3C)? as usize,
        width: u16_at(b, 0x44)? as u32,
        height: u16_at(b, 0x46)? as u32,
        compression: u8_at(b, 0x4A)?,
        mips: u8_at(b, 0x4E)?,
        tileable: u8_at(b, 0x4F)?,
        alpha_usage: u8_at(b, 0x55)?,
    })
}

fn parse_format(fmt: &[u8], compression: u8) -> Result<TexFormat> {
    let four_cc = u32_at(fmt, 0x14)?;
    Ok(match &four_cc.to_le_bytes() {
        b"DXT1" => TexFormat::Bc(Bc::Dxt1),
        b"DXT3" => TexFormat::Bc(Bc::Dxt3),
        b"DXT5" => TexFormat::Bc(Bc::Dxt5),
        _ => match four_cc {
            21 => TexFormat::Bgra8 { opaque: false },
            22 => TexFormat::Bgra8 { opaque: true },
            41 => TexFormat::P8,
            _ => match compression {
                0x22 => TexFormat::Bc(Bc::Dxt1),
                0x24 => TexFormat::Bc(Bc::Dxt3),
                0x26 => TexFormat::Bc(Bc::Dxt5),
                0x08 => TexFormat::P8,
                0x20 => TexFormat::Bgra8 { opaque: false },
                _ => TexFormat::Unknown(four_cc),
            },
        },
    })
}

fn make_texture(info: TexInfo, format: TexFormat, data: &[u8], palette_src: &[u8]) -> Result<Texture> {
    let pixels = slice(data, info.data_offset, info.data_size)?.to_vec();
    let palette = if format == TexFormat::P8 && info.palette_size > 0 {
        slice(palette_src, info.palette_offset, info.palette_size)?.to_vec()
    } else {
        Vec::new()
    };
    Ok(Texture {
        hash: info.hash,
        name: info.name,
        width: info.width,
        height: info.height,
        mip_count: (info.mips as u32).max(1),
        format,
        data: pixels,
        palette,
        alpha: match info.alpha_usage {
            0 => AlphaUsage::Opaque,
            1 => AlphaUsage::Cutout,
            _ => AlphaUsage::Blend,
        },
        tileable: info.tileable,
    })
}

impl Texture {
    /// Mip levels present in `data`: (width, height, bytes).
    pub fn mip_levels(&self) -> Vec<(u32, u32, &[u8])> {
        let mut out = Vec::new();
        let (mut w, mut h, mut off) = (self.width.max(1), self.height.max(1), 0usize);
        for _ in 0..self.mip_count {
            let size = match self.format {
                TexFormat::Bc(bc) => bc.level_size(w, h),
                TexFormat::Bgra8 { .. } => (w * h * 4) as usize,
                TexFormat::P8 => (w * h) as usize,
                TexFormat::Unknown(_) => break,
            };
            let Some(bytes) = self.data.get(off..off + size) else { break };
            out.push((w, h, bytes));
            off += size;
            if w == 1 && h == 1 {
                break;
            }
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
        out
    }

    /// Decodes a mip level to RGBA8.
    pub fn level_rgba8(&self, level: usize) -> Option<(u32, u32, Vec<u8>)> {
        let levels = self.mip_levels();
        let &(w, h, bytes) = levels.get(level)?;
        let rgba = match self.format {
            TexFormat::Bc(bc) => dxt::decode(bc, bytes, w, h),
            TexFormat::Bgra8 { opaque } => bytes
                .chunks_exact(4)
                .flat_map(|p| [p[2], p[1], p[0], if opaque { 255 } else { p[3] }])
                .collect(),
            TexFormat::P8 => bytes
                .iter()
                .flat_map(|&i| {
                    let o = i as usize * 4;
                    match self.palette.get(o..o + 4) {
                        Some(p) => [p[2], p[1], p[0], p[3]],
                        None => [255, 0, 255, 255],
                    }
                })
                .collect(),
            TexFormat::Unknown(_) => return None,
        };
        Some((w, h, rgba))
    }
}

/// Parses one TPK chunk. `buf` is the buffer `tpk.offset` is relative to.
pub fn parse_tpk(buf: &[u8], tpk: &Chunk) -> Result<TexturePack> {
    let Some(header) = tpk.child(ids::TPK_HEADER) else { bail!("TPK without header") };
    let mut pack = TexturePack::default();
    if let Some(info) = header.child(ids::TPK_INFO) {
        pack.name = cstr_at(info.data, 0x04, 28)?;
        pack.path = cstr_at(info.data, 0x20, 64)?;
        pack.hash = u32_at(info.data, 0x60)?;
    }

    if let Some(blocks) = header.child(ids::TPK_COMPRESSED_BLOCKS) {
        for e in blocks.data.chunks_exact(BLOCK_ENTRY_LEN) {
            let off = u32_at(e, 4)? as usize;
            let packed = u32_at(e, 8)? as usize;
            // Offsets are relative to the TPK start; in standalone files that is also 0.
            let src = [tpk.offset + off, off]
                .into_iter()
                .filter_map(|o| buf.get(o..o + packed))
                .find(|s| compression::is_compressed(s));
            let Some(src) = src else {
                log::warn!("{}: texture block {:08X} not found", pack.name, u32_at(e, 0)?);
                continue;
            };
            let block = match compression::decompress(src) {
                Ok(b) => b,
                Err(err) => {
                    log::warn!("{}: block {:08X}: {err}", pack.name, u32_at(e, 0)?);
                    continue;
                }
            };
            if block.len() < TEXTURE_INFO_LEN + FORMAT_INFO_LEN {
                continue;
            }
            let fmt_at = block.len() - FORMAT_INFO_LEN;
            let info_at = fmt_at - TEXTURE_INFO_LEN;
            let mut info = parse_info(&block[info_at..fmt_at])?;
            let format = parse_format(&block[fmt_at..], info.compression)?;
            // Offsets in the record refer to the original pack; in a block the pixels
            // come first, followed by the palette.
            info.data_offset = 0;
            info.palette_offset = info.data_size;
            let name = info.name.clone();
            match make_texture(info, format, &block[..info_at], &block[..info_at]) {
                Ok(t) => pack.textures.push(t),
                Err(e) => log::warn!("{}: texture {name}: {e}", pack.name),
            }
        }
        return Ok(pack);
    }

    let (Some(infos), Some(formats)) = (header.child(ids::TPK_TEXTURE_INFOS), header.child(ids::TPK_FORMATS))
    else {
        bail!("TPK {} has neither block table nor texture infos", pack.name)
    };
    let Some(blob) = tpk.child(ids::TPK_DATA).and_then(|d| d.child(ids::TPK_DATA_BLOB)) else {
        bail!("TPK {} has no data", pack.name)
    };
    // The blob is padded with 0x11 so that pixel data starts 0x80-aligned in the file.
    let pad = (0x80 - blob.body_offset() % 0x80) % 0x80;
    let data = if blob.data.len() >= pad && blob.data[..pad].iter().all(|&b| b == 0x11) {
        &blob.data[pad..]
    } else {
        blob.payload()
    };
    let n = infos.data.len() / TEXTURE_INFO_LEN;
    for i in 0..n {
        let info = parse_info(&infos.data[i * TEXTURE_INFO_LEN..])?;
        let fmt = slice(formats.data, i * FORMAT_INFO_LEN, FORMAT_INFO_LEN)?;
        let format = parse_format(fmt, info.compression)?;
        let name = info.name.clone();
        match make_texture(info, format, data, data) {
            Ok(t) => pack.textures.push(t),
            Err(e) => log::warn!("{}: texture {name}: {e}", pack.name),
        }
    }
    Ok(pack)
}

/// Finds and parses every TPK in `buf` (top level or nested one container deep).
pub fn find_tpks(buf: &[u8]) -> Vec<TexturePack> {
    let mut out = Vec::new();
    for c in ChunkIter::new(buf) {
        if c.id == ids::TPK {
            match parse_tpk(buf, &c) {
                Ok(p) => out.push(p),
                Err(e) => log::warn!("TPK at {:#x}: {e}", c.offset),
            }
        } else if c.is_container() {
            for cc in c.children().filter(|cc| cc.id == ids::TPK) {
                match parse_tpk(buf, &cc) {
                    Ok(p) => out.push(p),
                    Err(e) => log::warn!("TPK at {:#x}: {e}", cc.offset),
                }
            }
        }
    }
    out
}
