//! EA audio as shipped with Underground 2 (PC): sound banks, granular engine
//! recordings and the streamed music file.
//!
//! * `SOUND/**/*.abk` — "ABKC" AEMS banks. Besides the (x86) module code they embed a
//!   "BNKl" v5 bank at the offset stored at +0x20. The bank's table at +0x14 holds one
//!   u32 per sound, relative to its own slot (0 = empty slot, usually the first), each
//!   pointing at a `PT` header. Sample data offsets in the header are relative to the
//!   BNKl start. Every UG2 bank sound is mono EA-XA v2.
//! * `SOUND/ENGINE/GIN_*.gin` — "Gnsu20" granular engines: one recording of the engine
//!   sweeping from idle to redline, cut into grains, plus a curve giving the sample
//!   position for evenly spaced RPM steps. `*_DCL.gin` are the off-throttle sweeps.
//!   Audio is EA-XAS v0 (0x13-byte frames of 32 samples).
//! * `pfdata/MusicSFx.mus` — "SCHl" streams back to back, each starting 0x80-aligned:
//!   `SCHl` (PT header), `SCCl` (block count), `SCDl`* (audio), `SCEl`. A SCDl block is
//!   `"SCDl", u32 size, u32 samples, u32 channel offsets[channels]`, data at
//!   `0x0C + 4 * channels`. Two codecs occur: EA-XA (one mono stream per channel, each
//!   starting with its i16 LE history pair) and EALayer3 (MP3 granules, all channels in
//!   one bitstream at the first channel offset). EALayer3 is turned into standard MPEG
//!   Layer III frames here; decoding those needs an MP3 decoder.
//!
//! The `PT` header is a tag list: `u8 tag, u8 len, len bytes big-endian value`;
//! 0xFC/0xFD/0xFE open sub-sections, 0xFF ends it. Tags used: 0x80 version,
//! 0x82 channels (default 1), 0x84 sample rate (default 22050 on PC), 0x85 samples,
//! 0x86/0x87 loop start/end (inclusive), 0x88 data offset, 0xA0 codec (absent = EA-XA).

use std::io::{Read, Seek, SeekFrom};

use crate::bytes::{f32_at, i16_at, slice, u16_at, u32_at, u8_at};
use crate::{bail, Result};

/// ADPCM predictor pairs shared by EA-XA and EA-XA(S): coef1 = T[i], coef2 = T[i + 4].
const EA_XA_TABLE: [i32; 8] = [0, 240, 460, 392, 0, 0, -208, -220];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    /// EA-XA v2 (EA's CD-XA derivative, 28 samples per 15-byte frame, 0xEE PCM frames).
    EaXa,
    /// EALayer3: MP3 granules re-packed by EA (`0xA0` = 0x17).
    EaLayer3,
    Unknown(u32),
}

/// A parsed `PT` variable header.
#[derive(Clone, Debug, Default)]
pub struct PtHeader {
    pub platform: u16,
    tags: Vec<(u8, u32)>,
}

impl PtHeader {
    /// Parses the header at `o`; returns it and the offset just past the 0xFF end tag.
    pub fn parse(b: &[u8], o: usize) -> Result<(PtHeader, usize)> {
        if slice(b, o, 2)? != b"PT" {
            bail!("no PT header at {o:#x}");
        }
        let mut h = PtHeader { platform: u16_at(b, o + 2)?, tags: Vec::new() };
        let mut p = o + 4;
        loop {
            let tag = u8_at(b, p)?;
            p += 1;
            match tag {
                0xFF => break,
                0xFC..=0xFE => continue,
                _ => {
                    let len = u8_at(b, p)? as usize;
                    let v = slice(b, p + 1, len)?.iter().fold(0u32, |a, &x| (a << 8) | x as u32);
                    p += 1 + len;
                    if h.get(tag).is_none() {
                        h.tags.push((tag, v));
                    }
                }
            }
        }
        Ok((h, p))
    }

    pub fn get(&self, tag: u8) -> Option<u32> {
        self.tags.iter().find(|t| t.0 == tag).map(|t| t.1)
    }

    pub fn sample_rate(&self) -> u32 {
        self.get(0x84).unwrap_or(22050)
    }

    pub fn channels(&self) -> usize {
        self.get(0x82).unwrap_or(1) as usize
    }

    pub fn samples(&self) -> u32 {
        self.get(0x85).unwrap_or(0)
    }

    pub fn codec(&self) -> Codec {
        match self.get(0xA0) {
            None | Some(0x0A) => Codec::EaXa,
            Some(0x17) => Codec::EaLayer3,
            Some(c) => Codec::Unknown(c),
        }
    }
}

// ------------------------------------------------------------------ EA-XA

/// Decodes `n` mono EA-XA v2 samples starting at `off`, appending to `out`.
/// `hist` is (hist1, hist2) on entry and the decoder state on exit. Returns the offset
/// after the last frame read.
pub fn decode_ea_xa(b: &[u8], off: usize, n: usize, hist: &mut (i32, i32), out: &mut Vec<i16>) -> Result<usize> {
    let (mut h1, mut h2) = *hist;
    let target = out.len() + n;
    let mut p = off;
    while out.len() < target {
        let info = u8_at(b, p)?;
        if info == 0xEE {
            // Uncompressed frame: big-endian history pair and 28 big-endian samples.
            let f = slice(b, p + 5, 56)?;
            h1 = i16::from_be_bytes([b[p + 1], b[p + 2]]) as i32;
            h2 = i16::from_be_bytes([b[p + 3], b[p + 4]]) as i32;
            for s in f.chunks_exact(2).take(target - out.len()) {
                out.push(i16::from_be_bytes([s[0], s[1]]));
            }
            p += 61;
            continue;
        }
        let k = (info >> 4) as usize & 3;
        let (c1, c2) = (EA_XA_TABLE[k], EA_XA_TABLE[k + 4]);
        let shift = (info & 0x0F) as u32 + 8;
        let f = slice(b, p + 1, 14)?;
        for i in 0..28 {
            if out.len() == target {
                break;
            }
            let byte = f[i / 2];
            let nib = if i % 2 == 0 { byte >> 4 } else { byte & 0x0F };
            let s = ((nib as i32) << 28) >> shift; // sign-extend the nibble, then scale
            let s = ((s + c1 * h1 + c2 * h2) >> 8).clamp(-32768, 32767);
            out.push(s as i16);
            h2 = h1;
            h1 = s;
        }
        p += 15;
    }
    *hist = (h1, h2);
    Ok(p)
}

/// Decodes `n` mono EA-XAS v0 samples (GIN files). Each 0x13-byte frame is a u32 LE
/// header — bits 0-3 predictor, 4-15 hist2 (top 12 bits), 16-19 shift, 20-31 hist1 —
/// followed by 15 bytes of nibbles; the two history values are the first two samples.
pub fn decode_ea_xas_v0(b: &[u8], off: usize, n: usize) -> Result<Vec<i16>> {
    let mut out = Vec::with_capacity(n);
    let mut p = off;
    while out.len() < n {
        let hdr = u32_at(b, p)?;
        let f = slice(b, p + 4, 15)?;
        let k = (hdr & 3) as usize;
        let (c1, c2) = (EA_XA_TABLE[k], EA_XA_TABLE[k + 4]);
        let mut h2 = (hdr & 0xFFF0) as u16 as i16 as i32;
        let mut h1 = ((hdr >> 16) & 0xFFF0) as u16 as i16 as i32;
        let shift = 20 - ((hdr >> 16) & 0x0F);
        out.push(h2 as i16);
        out.push(h1 as i16);
        for i in 0..30 {
            let byte = f[i / 2];
            let nib = if i % 2 == 0 { byte >> 4 } else { byte & 0x0F };
            let nib = ((nib as i32) << 28) >> 28;
            let s = ((nib << shift) + c1 * h1 + c2 * h2 + 128) >> 8;
            let s = s.clamp(-32768, 32767);
            out.push(s as i16);
            h2 = h1;
            h1 = s;
        }
        p += 0x13;
    }
    out.truncate(n);
    Ok(out)
}

// ------------------------------------------------------------------ banks

#[derive(Clone, Debug)]
pub struct BankSound {
    /// Slot in the bank (the AEMS modules address sounds by slot).
    pub index: usize,
    pub sample_rate: u32,
    pub channels: usize,
    pub samples: u32,
    /// Inclusive loop range in samples, for looping sounds.
    pub looping: Option<(u32, u32)>,
    /// Absolute offset of the sample data in the buffer that was parsed.
    pub data: usize,
    pub codec: Codec,
}

impl BankSound {
    pub fn decode(&self, buf: &[u8]) -> Result<Vec<i16>> {
        if self.codec != Codec::EaXa || self.channels != 1 {
            bail!("bank sound {}: unsupported {:?} x{}", self.index, self.codec, self.channels);
        }
        let mut out = Vec::with_capacity(self.samples as usize);
        decode_ea_xa(buf, self.data, self.samples as usize, &mut (0, 0), &mut out)?;
        Ok(out)
    }
}

/// Parses the "BNKl" bank at `base`. Empty slots are skipped; several slots may share data.
pub fn parse_bnk(b: &[u8], base: usize) -> Result<Vec<BankSound>> {
    if slice(b, base, 4)? != b"BNKl" {
        bail!("no BNKl at {base:#x}");
    }
    let count = u16_at(b, base + 6)? as usize;
    let mut out = Vec::new();
    for i in 0..count {
        let slot = base + 0x14 + 4 * i;
        let rel = u32_at(b, slot)? as usize;
        if rel == 0 {
            continue;
        }
        let (h, _) = PtHeader::parse(b, slot + rel)?;
        out.push(BankSound {
            index: i,
            sample_rate: h.sample_rate(),
            channels: h.channels(),
            samples: h.samples(),
            looping: h.get(0x86).zip(h.get(0x87)),
            data: base + h.get(0x88).unwrap_or(0) as usize,
            codec: h.codec(),
        });
    }
    Ok(out)
}

/// Sounds of an "ABKC" file (the embedded BNKl).
pub fn parse_abk(b: &[u8]) -> Result<Vec<BankSound>> {
    if slice(b, 0, 4)? != b"ABKC" {
        bail!("not an ABK file");
    }
    parse_bnk(b, u32_at(b, 0x20)? as usize)
}

// ------------------------------------------------------------------ GIN engines

/// A granular engine recording (`Gnsu20`).
#[derive(Clone, Debug)]
pub struct Gin {
    pub min_rpm: f32,
    pub max_rpm: f32,
    pub sample_rate: u32,
    /// Grain boundaries: grain `i` is `pcm[grains[i]..grains[i + 1]]`.
    pub grains: Vec<u32>,
    /// Sample position of the sweep at `min_rpm + i * (max_rpm - min_rpm) / (len - 1)`.
    pub rpm_curve: Vec<u32>,
    pub pcm: Vec<i16>,
}

impl Gin {
    pub fn parse(b: &[u8]) -> Result<Gin> {
        if slice(b, 0, 4)? != b"Gnsu" {
            bail!("not a GIN file");
        }
        let n_grains = u32_at(b, 0x10)? as usize + 1;
        let n_curve = u32_at(b, 0x14)? as usize + 1;
        let samples = u32_at(b, 0x18)? as usize;
        let words = |o: usize, n: usize| (0..n).map(|i| u32_at(b, o + 4 * i)).collect::<Result<Vec<_>>>();
        let grains = words(0x20, n_grains)?;
        let curve_at = 0x20 + 4 * n_grains;
        let rpm_curve = words(curve_at, n_curve)?;
        let pcm = decode_ea_xas_v0(b, curve_at + 4 * n_curve, samples)?;
        Ok(Gin {
            min_rpm: f32_at(b, 8)?,
            max_rpm: f32_at(b, 0x0C)?,
            sample_rate: u32_at(b, 0x1C)?,
            grains,
            rpm_curve,
            pcm,
        })
    }

    /// Position (in samples) of the sweep where the engine runs at `rpm`.
    pub fn position(&self, rpm: f32) -> f32 {
        let n = self.rpm_curve.len();
        if n < 2 || self.max_rpm <= self.min_rpm {
            return 0.0;
        }
        let t = ((rpm - self.min_rpm) / (self.max_rpm - self.min_rpm)).clamp(0.0, 1.0) * (n - 1) as f32;
        let i = (t as usize).min(n - 2);
        let f = t - i as f32;
        self.rpm_curve[i] as f32 * (1.0 - f) + self.rpm_curve[i + 1] as f32 * f
    }

    /// Index of the grain containing sample position `pos`.
    pub fn grain_at(&self, pos: f32) -> usize {
        let p = pos.max(0.0) as u32;
        self.grains.partition_point(|&g| g <= p).saturating_sub(1).min(self.grains.len().saturating_sub(2))
    }

    /// Samples of grain `i`; empty when out of range or when the boundaries are not
    /// ascending (a few recordings have such entries).
    pub fn grain(&self, i: usize) -> &[i16] {
        let (Some(&a), Some(&b)) = (self.grains.get(i), self.grains.get(i + 1)) else { return &[] };
        let b = (b as usize).min(self.pcm.len());
        let a = (a as usize).min(b);
        &self.pcm[a..b]
    }
}

// ------------------------------------------------------------------ SCHl streams

#[derive(Clone, Debug)]
pub struct StreamHeader {
    pub sample_rate: u32,
    pub channels: usize,
    pub samples: u32,
    pub codec: Codec,
}

impl StreamHeader {
    fn from_pt(h: &PtHeader) -> StreamHeader {
        StreamHeader { sample_rate: h.sample_rate(), channels: h.channels(), samples: h.samples(), codec: h.codec() }
    }
}

fn read_block_header<R: Read + Seek>(r: &mut R, at: u64) -> Result<Option<([u8; 4], u32)>> {
    r.seek(SeekFrom::Start(at))?;
    let mut h = [0u8; 8];
    match r.read_exact(&mut h) {
        Ok(()) => Ok(Some(([h[0], h[1], h[2], h[3]], u32::from_le_bytes([h[4], h[5], h[6], h[7]])))),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Sequential reader for one SCHl stream inside a larger file (e.g. MusicSFx.mus).
pub struct SchlReader<R> {
    r: R,
    pos: u64,
    pub header: StreamHeader,
    done: bool,
}

impl<R: Read + Seek> SchlReader<R> {
    pub fn open(mut r: R, offset: u64) -> Result<Self> {
        let Some((tag, size)) = read_block_header(&mut r, offset)? else { bail!("no stream at {offset:#x}") };
        if &tag != b"SCHl" || size < 12 {
            bail!("no SCHl at {offset:#x}");
        }
        let mut b = vec![0u8; size as usize];
        r.seek(SeekFrom::Start(offset))?;
        r.read_exact(&mut b)?;
        let (h, _) = PtHeader::parse(&b, 8)?;
        Ok(Self { r, pos: offset + size as u64, header: StreamHeader::from_pt(&h), done: false })
    }

    /// Reads the next SCDl block (whole block, header included) into `buf`.
    /// Returns false at the end of the stream.
    pub fn next_block(&mut self, buf: &mut Vec<u8>) -> Result<bool> {
        while !self.done {
            let Some((tag, size)) = read_block_header(&mut self.r, self.pos)? else { break };
            if size < 8 {
                bail!("bad stream block size {size} at {:#x}", self.pos);
            }
            let at = self.pos;
            self.pos += size as u64;
            match &tag {
                b"SCDl" => {
                    buf.resize(size as usize, 0);
                    self.r.seek(SeekFrom::Start(at))?;
                    self.r.read_exact(buf)?;
                    return Ok(true);
                }
                b"SCEl" => self.done = true,
                b"SCCl" | b"SCLl" => {}
                _ => bail!("unexpected stream block {:?} at {at:#x}", String::from_utf8_lossy(&tag)),
            }
        }
        self.done = true;
        Ok(false)
    }

    /// File offset just past this stream's SCEl block (valid once `next_block` returned false).
    pub fn end(&self) -> u64 {
        self.pos
    }
}

/// Lists every SCHl stream in a file of concatenated streams (each 0x80-aligned).
/// Walks block headers only (~150k small reads for the 500 MB MusicSFx.mus), so cache it.
pub fn scan_streams<R: Read + Seek>(r: &mut R) -> Result<Vec<(u64, StreamHeader)>> {
    let len = r.seek(SeekFrom::End(0))?;
    let mut out = Vec::new();
    let mut at = 0u64;
    while at + 8 <= len {
        match read_block_header(r, at)? {
            Some((tag, _)) if &tag == b"SCHl" => {}
            _ => {
                at = (at + 0x80) & !0x7F;
                continue;
            }
        }
        let header = SchlReader::open(&mut *r, at)?.header;
        let mut pos = at;
        while let Some((tag, size)) = read_block_header(r, pos)? {
            if size < 8 {
                break;
            }
            pos += size as u64;
            if &tag == b"SCEl" {
                break;
            }
        }
        out.push((at, header));
        at = (pos + 0x7F) & !0x7F;
    }
    Ok(out)
}

/// Decodes one EA-XA SCDl block to interleaved PCM.
pub fn decode_ea_xa_block(block: &[u8], channels: usize, out: &mut Vec<i16>) -> Result<()> {
    let n = u32_at(block, 8)? as usize;
    let base = 0x0C + 4 * channels;
    let mut planes = Vec::with_capacity(channels);
    for c in 0..channels {
        let p = base + u32_at(block, 0x0C + 4 * c)? as usize;
        let mut hist = (i16_at(block, p)? as i32, i16_at(block, p + 2)? as i32);
        let mut v = Vec::with_capacity(n);
        decode_ea_xa(block, p + 4, n, &mut hist, &mut v)?;
        planes.push(v);
    }
    out.reserve(n * channels);
    for i in 0..n {
        for p in &planes {
            out.push(p[i]);
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ EALayer3

struct BitReader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl BitReader<'_> {
    fn read(&mut self, n: u32) -> Result<u64> {
        let mut v = 0u64;
        for _ in 0..n {
            let Some(&byte) = self.b.get(self.pos >> 3) else { bail!("EALayer3 frame truncated") };
            v = (v << 1) | ((byte >> (7 - (self.pos & 7))) & 1) as u64;
            self.pos += 1;
        }
        Ok(v)
    }
}

#[derive(Default)]
struct BitWriter {
    out: Vec<u8>,
    acc: u8,
    n: u32,
}

impl BitWriter {
    fn write(&mut self, n: u32, v: u64) {
        for i in (0..n).rev() {
            self.acc = (self.acc << 1) | ((v >> i) & 1) as u8;
            self.n += 1;
            if self.n == 8 {
                self.out.push(self.acc);
                self.acc = 0;
                self.n = 0;
            }
        }
    }

    fn align(&mut self) {
        while self.n != 0 {
            self.write(1, 0);
        }
    }
}

/// One MP3 granule as stored by EA: the frame header fields, the side info split into
/// `part2_3_length` (12 bits, the main data size) and the remaining 47/51 bits, and the
/// main data bits themselves.
struct Granule {
    version_index: u8,
    sr_index: u8,
    channel_mode: u8,
    mode_ext: u8,
    index: u8,
    channels: usize,
    scfsi: [u8; 2],
    side: [(u16, u64); 2],
    data: BitWriter,
}

impl Granule {
    fn mpeg1(&self) -> bool {
        self.version_index == 3
    }

    fn rate(&self) -> u32 {
        const RATES: [[u32; 3]; 4] = [[11025, 12000, 8000], [0, 0, 0], [22050, 24000, 16000], [44100, 48000, 32000]];
        RATES[self.version_index as usize][(self.sr_index as usize).min(2)]
    }
}

/// Converts EALayer3 (v1, as used by SCHl codec 0x17) to standard MPEG Layer III frames.
///
/// Each EA frame starts with a flag byte: 0x00 = one granule
/// (2b version, 2b rate, 2b channel mode, 2b mode extension, 1b granule index,
/// 4b scfsi per channel on MPEG-1 granule 1, then per channel 12b part2_3_length and
/// 47b/51b of the remaining side info, the main data, zero padding to a byte);
/// 0xEE = PCM block (u16 BE samples to drop from the following decoded output,
/// u16 BE count, count x channels BE i16 interleaved) — at the start of a stream it
/// carries no granule. Frames are rebuilt without bit reservoir (main_data_begin = 0)
/// at the smallest standard bitrate that holds them.
#[derive(Default)]
pub struct EaLayer3 {
    pending: Option<Granule>,
    version_index: Option<u8>,
    /// PCM blocks found so far: (samples to drop from the following MP3 output, interleaved PCM).
    pub pcm_blocks: Vec<(u32, Vec<i16>)>,
    /// Frames that did not fit any standard bitrate and were written as free format.
    pub free_format_frames: usize,
}

impl EaLayer3 {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends the MP3 frames of one SCDl block to `mp3`.
    pub fn push_block(&mut self, block: &[u8], channels: usize, mp3: &mut Vec<u8>) -> Result<()> {
        let start = 0x0C + 4 * channels + u32_at(block, 0x0C)? as usize;
        let Some(data) = block.get(start..) else { bail!("EALayer3 block too short") };
        let mut r = BitReader { b: data, pos: 0 };
        while (r.pos >> 3) + 2 <= data.len() {
            let flag = data[r.pos >> 3];
            let version = data[(r.pos >> 3) + 1] >> 6;
            // Blocks are zero-padded: a "granule" whose MPEG version differs from the
            // stream's, or one that runs past the block, is padding.
            if (flag != 0x00 && flag != 0xEE) || (flag == 0 && self.version_index.is_some_and(|v| v != version)) {
                break;
            }
            match self.parse_frame(&mut r, channels) {
                Ok(Some(g)) => self.push_granule(g, mp3),
                Ok(None) => {}
                Err(_) => break,
            }
        }
        Ok(())
    }

    fn parse_frame(&mut self, r: &mut BitReader, channels: usize) -> Result<Option<Granule>> {
        let start = r.pos;
        let flag = r.read(8)?;
        let mut granule = None;
        let peek_version = {
            let save = r.pos;
            let v = r.read(2)? as u8;
            r.pos = save;
            v
        };
        let has_granule = flag == 0 || (peek_version != 1 && self.version_index.is_none_or(|v| v == peek_version));
        if has_granule {
            let version_index = r.read(2)? as u8;
            if version_index == 1 {
                bail!("EALayer3: reserved MPEG version");
            }
            let sr_index = r.read(2)? as u8;
            let channel_mode = r.read(2)? as u8;
            let mode_ext = r.read(2)? as u8;
            let index = r.read(1)? as u8;
            let ch = if channel_mode == 3 { 1 } else { 2 };
            let mpeg1 = version_index == 3;
            let mut scfsi = [0u8; 2];
            if mpeg1 && index == 1 {
                for s in scfsi.iter_mut().take(ch) {
                    *s = r.read(4)? as u8;
                }
            }
            let other_bits = if mpeg1 { 47 } else { 51 };
            let mut side = [(0u16, 0u64); 2];
            for s in side.iter_mut().take(ch) {
                *s = (r.read(12)? as u16, r.read(other_bits)?);
            }
            let header_bits = r.pos - start - 8;
            let mut data = BitWriter::default();
            let mut bits: usize = side.iter().take(ch).map(|s| s.0 as usize).sum();
            let data_bits = bits;
            while bits > 0 {
                let n = bits.min(32) as u32;
                data.write(n, r.read(n)?);
                bits -= n as usize;
            }
            r.pos += (8 - (header_bits + data_bits) % 8) % 8;
            self.version_index = Some(version_index);
            granule = Some(Granule { version_index, sr_index, channel_mode, mode_ext, index, channels: ch, scfsi, side, data });
        }
        if flag == 0xEE {
            let discard = r.read(16)? as u32;
            let count = r.read(16)? as usize * channels;
            let at = r.pos >> 3;
            let Some(raw) = r.b.get(at..at + count * 2) else { bail!("EALayer3 PCM block truncated") };
            self.pcm_blocks.push((discard, raw.chunks_exact(2).map(|s| i16::from_be_bytes([s[0], s[1]])).collect()));
            r.pos += count * 16;
        }
        Ok(granule)
    }

    fn push_granule(&mut self, g: Granule, mp3: &mut Vec<u8>) {
        if !g.mpeg1() {
            self.write_frame(&[&g], mp3);
            return;
        }
        match (self.pending.take(), g.index) {
            (Some(g0), 1) => self.write_frame(&[&g0, &g], mp3),
            (_, 0) => self.pending = Some(g),
            (None, _) => {} // granule 1 without its granule 0: resync
            (Some(_), _) => {}
        }
    }

    fn write_frame(&mut self, gr: &[&Granule], mp3: &mut Vec<u8>) {
        let g0 = gr[0];
        let ch = g0.channels;
        let mpeg1 = g0.mpeg1();
        let mut body = BitWriter::default();
        if mpeg1 {
            body.write(9, 0); // main_data_begin
            body.write(if ch == 1 { 5 } else { 3 }, 0);
            for c in 0..ch {
                body.write(4, gr[1].scfsi[c] as u64);
            }
        } else {
            body.write(8, 0);
            body.write(if ch == 1 { 1 } else { 2 }, 0);
        }
        let other_bits = if mpeg1 { 47 } else { 51 };
        for g in gr {
            for c in 0..ch {
                body.write(12, g.side[c].0 as u64);
                body.write(other_bits, g.side[c].1);
            }
        }
        for g in gr {
            for &byte in &g.data.out {
                body.write(8, byte as u64);
            }
            body.write(g.data.n, g.data.acc as u64);
        }
        body.align();

        let need = 4 + body.out.len();
        const MPEG1: [u32; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
        const MPEG2: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
        let (table, coef) = if mpeg1 { (&MPEG1, 144) } else { (&MPEG2, 72) };
        let rate = g0.rate().max(1);
        let pick = (1..15).map(|i| (i, (coef * table[i] * 1000 / rate) as usize)).find(|&(_, size)| size >= need);
        let (index, size) = pick.unwrap_or_else(|| {
            self.free_format_frames += 1;
            (0, need)
        });

        let mut h = BitWriter::default();
        h.write(11, 0x7FF);
        h.write(2, g0.version_index as u64);
        h.write(2, 1); // layer III
        h.write(1, 1); // no CRC
        h.write(4, index as u64);
        h.write(2, g0.sr_index as u64);
        h.write(1, 0); // padding
        h.write(1, 0); // private
        h.write(2, g0.channel_mode as u64);
        h.write(2, g0.mode_ext as u64);
        h.write(1, 0); // copyright
        h.write(1, 1); // original
        h.write(2, 0); // emphasis
        let at = mp3.len();
        mp3.extend_from_slice(&h.out);
        mp3.extend_from_slice(&body.out);
        mp3.resize(at + size, 0);
    }
}

// ------------------------------------------------------------------ engine tables

/// One engine sound set, from the table in the game executable (43 entries).
/// RPMs map the car's engine speed onto the GIN sweeps (the GIN headers carry the exact
/// recorded range); `volume` is the game's quiet/loud level on a 0..32767 scale.
#[derive(Clone, Copy, Debug)]
pub struct Engine {
    /// Looped samples at fixed pitches (used by the game for AI and traffic engines).
    pub abk: &'static str,
    /// On-throttle sweep.
    pub gin: &'static str,
    /// Off-throttle sweep.
    pub decel_gin: &'static str,
    pub idle_rpm: f32,
    pub max_rpm: f32,
    pub decel_rpm: (f32, f32),
    pub volume: (u16, u16),
}

/// Files in `SOUND/SHIFTING`, indexed by the gear value of [`CAR_SOUNDS`].
pub const GEARS: [&str; 16] = [
    "GEAR_SML_Base.abk", "GEAR_SML_Lev1.abk", "GEAR_SML_Lev2.abk", "GEAR_SML_Lev3.abk",
    "GEAR_MED_Base.abk", "GEAR_MED_Lev1.abk", "GEAR_MED_Lev2.abk", "GEAR_MED_Lev3.abk",
    "GEAR_LRG_Base.abk", "GEAR_LRG_Lev1.abk", "GEAR_LRG_Lev2.abk", "GEAR_LRG_Lev3.abk",
    "GEAR_TK_Base.abk", "GEAR_TK_Lev1.abk", "GEAR_TK_Lev2.abk", "GEAR_TK_Lev3.abk",
];

/// Files in `SOUND/TURBO`, indexed by the turbo value of [`CAR_SOUNDS`].
pub const TURBOS: [&str; 15] = [
    "TURBO_SML1_0_MB.abk", "TURBO_SML1_1_MB.abk", "TURBO_SML1_2_MB.abk",
    "TURBO_SML2_0_MB.abk", "TURBO_SML2_1_MB.abk", "TURBO_SML2_2_MB.abk",
    "TURBO_MED_0_MB.abk", "TURBO_MED_1_MB.abk", "TURBO_MED_2_MB.abk",
    "TURBO_BIG_0_MB.abk", "TURBO_BIG_1_MB.abk", "TURBO_BIG_2_MB.abk",
    "TURBO_TRUCK_0_MB.abk", "TURBO_TRUCK_1_MB.abk", "TURBO_TRUCK_2_MB.abk",
];

/// Engine, gear-shift and turbo sounds of a player car (`name` as in CarTypeInfo) at an
/// engine upgrade level 0..=3 (stock, level 1-3).
pub fn car_sounds(name: &str, level: usize) -> Option<(&'static Engine, &'static str, &'static str)> {
    let (_, levels) = CAR_SOUNDS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name))?;
    let (e, g, t) = levels[level.min(3)];
    Some((ENGINES.get(e as usize)?, GEARS.get(g as usize)?, TURBOS.get(t as usize)?))
}

pub const ENGINES: [Engine; 43] = [
    Engine { abk: "CAR_00_ENG_MB_EE.abk", gin: "GIN_HondaS2000_Church.gin", decel_gin: "GIN_S2000_Church_DCL.gin", idle_rpm: 1350.0, max_rpm: 9229.0, decel_rpm: (3692.0, 9182.0), volume: (18000, 20000) },
    Engine { abk: "CAR_01_ENG_MB_EE.abk", gin: "GIN_Ferrari_360.gin", decel_gin: "GIN_Ferrari_360_DCL.gin", idle_rpm: 1500.0, max_rpm: 8222.0, decel_rpm: (2072.0, 8273.0), volume: (16000, 23000) },
    Engine { abk: "CAR_02_ENG_MB_EE.abk", gin: "GIN_Trueno_AE86.gin", decel_gin: "GIN_Trueno_DCL.gin", idle_rpm: 1250.0, max_rpm: 8007.0, decel_rpm: (2019.0, 7532.0), volume: (16500, 24500) },
    Engine { abk: "CAR_03_ENG_MB_EE.abk", gin: "GIN_Honda_S2000_5zigen.gin", decel_gin: "GIN_Honda_S2000_5zigen_DCL.gin", idle_rpm: 1250.0, max_rpm: 8311.0, decel_rpm: (1365.0, 8033.0), volume: (16000, 28500) },
    Engine { abk: "CAR_04_ENG_MB_EE.abk", gin: "GIN_Honda_S2000_Spugen.gin", decel_gin: "GIN_Honda_S2000_Spugen_DCL.gin", idle_rpm: 1000.0, max_rpm: 8836.0, decel_rpm: (2415.0, 8084.0), volume: (16500, 29000) },
    Engine { abk: "CAR_05_ENG_MB_EE.abk", gin: "GIN_Honda_S2000_spliced.gin", decel_gin: "GIN_Honda_S2000_2f2f_DCL.gin", idle_rpm: 1000.0, max_rpm: 8338.0, decel_rpm: (4350.0, 8457.0), volume: (16000, 24000) },
    Engine { abk: "CAR_06_ENG_MB_EE.abk", gin: "GIN_MitsEclipse_2000.gin", decel_gin: "GIN_Eclipse_2000_DCL.gin", idle_rpm: 1000.0, max_rpm: 6888.0, decel_rpm: (2502.0, 6829.0), volume: (16500, 24500) },
    Engine { abk: "CAR_07_ENG_MB_EE.abk", gin: "GIN_Nissan_300ZX_Street.gin", decel_gin: "GIN_300ZX_DCL.gin", idle_rpm: 800.0, max_rpm: 7099.0, decel_rpm: (1701.0, 7079.0), volume: (16500, 28500) },
    Engine { abk: "CAR_08_ENG_MB_EE.abk", gin: "GIN_Eclipse_5zigen.gin", decel_gin: "GIN_Mits_5zigen_DCL.gin", idle_rpm: 1000.0, max_rpm: 7145.0, decel_rpm: (1826.0, 7097.0), volume: (16500, 24500) },
    Engine { abk: "CAR_09_ENG_MB_EE.abk", gin: "GIN_Nissan_Skyline.gin", decel_gin: "GIN_Mustang_Drag_DCL.gin", idle_rpm: 1450.0, max_rpm: 8292.0, decel_rpm: (2779.0, 6381.0), volume: (14500, 28500) },
    Engine { abk: "CAR_10_ENG_MB_EE.abk", gin: "GIN_Mazda_RX7.gin", decel_gin: "GIN_Mazda_RX7_DCL.gin", idle_rpm: 1100.0, max_rpm: 7239.0, decel_rpm: (1869.0, 7247.0), volume: (26000, 32000) },
    Engine { abk: "CAR_11_ENG_MB_EE.abk", gin: "GIN_RX8_Ramp_v1_CD.gin", decel_gin: "GIN_Mazda_RX8_DCL.gin", idle_rpm: 1250.0, max_rpm: 8936.0, decel_rpm: (5320.0, 8756.0), volume: (21500, 31500) },
    Engine { abk: "CAR_12_ENG_MB_EE.abk", gin: "GIN_Mustang_Drag.gin", decel_gin: "GIN_Mustang_Drag_DCL.gin", idle_rpm: 750.0, max_rpm: 6509.0, decel_rpm: (2779.0, 6381.0), volume: (18000, 28000) },
    Engine { abk: "CAR_13_ENG_MB_EE.abk", gin: "GIN_Bronco_Comp.gin", decel_gin: "GIN_Bronco_DCL.gin", idle_rpm: 750.0, max_rpm: 5672.0, decel_rpm: (4629.0, 5747.0), volume: (18000, 26000) },
    Engine { abk: "CAR_14_ENG_MB_EE.abk", gin: "GIN_MustSal_Ramp_V2_CD.gin", decel_gin: "GIN_Mustang_Saleen_DCL.gin", idle_rpm: 750.0, max_rpm: 5121.0, decel_rpm: (1493.0, 5388.0), volume: (18000, 26000) },
    Engine { abk: "CAR_15_ENG_MB_EE.abk", gin: "GIN_WRXSTi_Pull_09.gin", decel_gin: "GIN_WRX_STi_DCL.gin", idle_rpm: 1000.0, max_rpm: 6975.0, decel_rpm: (1218.0, 6888.0), volume: (22000, 32000) },
    Engine { abk: "CAR_16_ENG_MB_EE.abk", gin: "GIN_WRX_Street.gin", decel_gin: "GIN_WRX_Street_DCL.gin", idle_rpm: 1000.0, max_rpm: 6313.0, decel_rpm: (3916.0, 6339.0), volume: (18000, 32500) },
    Engine { abk: "CAR_17_ENG_MB_EE.abk", gin: "GIN_WRX_2F2F.gin", decel_gin: "GIN_WRX2F2F_DCL.gin", idle_rpm: 1000.0, max_rpm: 6881.0, decel_rpm: (4818.0, 6878.0), volume: (20000, 32000) },
    Engine { abk: "CAR_18_ENG_MB_EE.abk", gin: "GIN_Nissan_240SX.gin", decel_gin: "GIN_240SX_Decel.gin", idle_rpm: 1526.0, max_rpm: 7037.0, decel_rpm: (1120.0, 7012.0), volume: (18000, 28500) },
    Engine { abk: "CAR_19_ENG_MB_EE.abk", gin: "GIN_Toyota_GTS.gin", decel_gin: "GIN_Toyota_GTS_DCL.gin", idle_rpm: 1250.0, max_rpm: 8222.0, decel_rpm: (2099.0, 8077.0), volume: (16500, 24000) },
    Engine { abk: "CAR_20_ENG_MB_EE.abk", gin: "GIN_Honda_Prelude.gin", decel_gin: "GIN_Honda_Prelude_DCL.gin", idle_rpm: 1500.0, max_rpm: 7158.0, decel_rpm: (4960.0, 7001.0), volume: (16000, 24000) },
    Engine { abk: "CAR_21_ENG_MB_EE.abk", gin: "GIN_VW_Scirocco.gin", decel_gin: "GIN_VW_Scirocco_DCL.gin", idle_rpm: 1500.0, max_rpm: 7085.0, decel_rpm: (1831.0, 7115.0), volume: (20000, 28500) },
    Engine { abk: "CAR_22_ENG_MB_EE.abk", gin: "GIN_Honda_S2000_APEX.gin", decel_gin: "GIN_HONDA_S2000_Apex_DCL.gin", idle_rpm: 1250.0, max_rpm: 8284.0, decel_rpm: (2246.0, 7847.0), volume: (22500, 30000) },
    Engine { abk: "CAR_23_ENG_MB_EE.abk", gin: "GIN_Mazda_Miata.gin", decel_gin: "GIN_Mazda_Miata_DCL.gin", idle_rpm: 1500.0, max_rpm: 7158.0, decel_rpm: (2201.0, 6863.0), volume: (16500, 24000) },
    Engine { abk: "CAR_24_ENG_MB_EE.abk", gin: "GIN_VW_Drag_GTi.gin", decel_gin: "GIN_VW_Drag_GTi_DCL.gin", idle_rpm: 1750.0, max_rpm: 7363.0, decel_rpm: (2385.0, 7173.0), volume: (16000, 24000) },
    Engine { abk: "CAR_25_ENG_MB_EE.abk", gin: "GIN_G35_350ZX_Comp_v2_AB.gin", decel_gin: "GIN_Infinit_G35_DCL.gin", idle_rpm: 1200.0, max_rpm: 6599.0, decel_rpm: (1535.0, 6209.0), volume: (24000, 28000) },
    Engine { abk: "CAR_26_ENG_MB_EE.abk", gin: "GIN_350Z_01d_shrt2.gin", decel_gin: "GIN_Infinit_G35_DCL.gin", idle_rpm: 1200.0, max_rpm: 6510.0, decel_rpm: (1535.0, 6209.0), volume: (22000, 30000) },
    Engine { abk: "CAR_27_ENG_MB_EE.abk", gin: "GIN_350Z_Jason_shrt2.gin", decel_gin: "GIN_Nissan_350ZX_Jason_DCL.gin", idle_rpm: 1250.0, max_rpm: 6596.0, decel_rpm: (1560.0, 6415.0), volume: (18000, 28500) },
    Engine { abk: "CAR_28_ENG_MB_EE.abk", gin: "GIN_VR6_Jetta_M3.gin", decel_gin: "GIN_VW_Jetta_M3_DCL.gin", idle_rpm: 1000.0, max_rpm: 6773.0, decel_rpm: (1237.0, 6314.0), volume: (22000, 32500) },
    Engine { abk: "CAR_29_ENG_MB_EE.abk", gin: "GIN_VW_Cabriolet.gin", decel_gin: "GIN_VW_Cab_DCL.gin", idle_rpm: 1350.0, max_rpm: 5653.0, decel_rpm: (4567.0, 5630.0), volume: (18000, 32500) },
    Engine { abk: "CAR_30_ENG_MB_EE.abk", gin: "GIN_VR6_Jetta_M4.gin", decel_gin: "GIN_VW_Jetta_M4_DCL.gin", idle_rpm: 1250.0, max_rpm: 6548.0, decel_rpm: (1472.0, 6480.0), volume: (20000, 28500) },
    Engine { abk: "CAR_31_ENG_MB_EE.abk", gin: "GIN_FordFocus_Road.gin", decel_gin: "GIN_Ford_Focus2_DCL.gin", idle_rpm: 1150.0, max_rpm: 6566.0, decel_rpm: (2456.0, 6818.0), volume: (16000, 22000) },
    Engine { abk: "CAR_32_ENG_MB_EE.abk", gin: "GIN_FordFocus_comp2.gin", decel_gin: "GIN_Ford_Focus2_DCL.gin", idle_rpm: 1250.0, max_rpm: 6840.0, decel_rpm: (2456.0, 6818.0), volume: (22000, 30000) },
    Engine { abk: "CAR_33_ENG_MB_EE.abk", gin: "GIN_Protege_Ramp_V2_CD.gin", decel_gin: "GIN_Protege_Lev2_DCL.gin", idle_rpm: 1200.0, max_rpm: 6786.0, decel_rpm: (1498.0, 6561.0), volume: (18000, 24000) },
    Engine { abk: "CAR_34_ENG_MB_EE.abk", gin: "GIN_Protege_Ramp_Lev2_CD.gin", decel_gin: "GIN_Protege_Lev2_DCL.gin", idle_rpm: 1250.0, max_rpm: 7301.0, decel_rpm: (1973.0, 6960.0), volume: (18000, 26500) },
    Engine { abk: "CAR_35_ENG_MB_EE.abk", gin: "GIN_Mustang_Nscr_COMP.gin", decel_gin: "GIN_Nascar_DCL.gin", idle_rpm: 1250.0, max_rpm: 7421.0, decel_rpm: (3798.0, 7118.0), volume: (22000, 32500) },
    Engine { abk: "CAR_36_ENG_MB_EE.abk", gin: "GIN_Cavel_v1_Base.gin", decel_gin: "GIN_Cavel_DCL.gin", idle_rpm: 1250.0, max_rpm: 6508.0, decel_rpm: (1844.0, 7103.0), volume: (12500, 28500) },
    Engine { abk: "CAR_37_ENG_MB_EE.abk", gin: "GIN_Cavel_v1_Lev1.gin", decel_gin: "GIN_Cavel_DCL.gin", idle_rpm: 1000.0, max_rpm: 6958.0, decel_rpm: (1844.0, 7103.0), volume: (13500, 24000) },
    Engine { abk: "CAR_38_ENG_MB_EE.abk", gin: "GIN_Hummer.gin", decel_gin: "GIN_Bronco_DCL.gin", idle_rpm: 1000.0, max_rpm: 4782.0, decel_rpm: (4629.0, 5747.0), volume: (17500, 32500) },
    Engine { abk: "CAR_39_ENG_MB_EE.abk", gin: "GIN_Porsche_Cayenne.gin", decel_gin: "GIN_Porsche_Cayenne_DCL.gin", idle_rpm: 1000.0, max_rpm: 7052.0, decel_rpm: (3842.0, 6060.0), volume: (16500, 28000) },
    Engine { abk: "CAR_40_ENG_MB_EE.abk", gin: "GIN_Acura_ITR.gin", decel_gin: "GIN_Acura_ITR_DCL.gin", idle_rpm: 1250.0, max_rpm: 8326.0, decel_rpm: (2528.0, 8631.0), volume: (15000, 24000) },
    Engine { abk: "CAR_14_ENG_MB_EE.abk", gin: "GIN_MustSal_Ramp_V2_CD.gin", decel_gin: "GIN_Mustang_Saleen_DCL.gin", idle_rpm: 750.0, max_rpm: 5200.0, decel_rpm: (1493.0, 5388.0), volume: (18000, 24500) },
    Engine { abk: "CAR_12_ENG_MB_EE.abk", gin: "GIN_Mustang_Drag.gin", decel_gin: "GIN_Mustang_Drag_DCL.gin", idle_rpm: 750.0, max_rpm: 6509.0, decel_rpm: (2779.0, 6381.0), volume: (16000, 26000) },
];

/// Per player car (CarTypeInfo order, +0x840 index), per upgrade level 0..=3:
/// (engine, gear-shift bank, turbo bank) indices into ENGINES, GEARS and TURBOS.
pub const CAR_SOUNDS: [(&str, [(u8, u8, u8); 4]); 31] = [
    ("PEUGOT", [(23, 0, 3), (36, 1, 3), (37, 2, 4), (37, 3, 5)]),
    ("FOCUS", [(31, 0, 0), (32, 1, 0), (36, 2, 1), (37, 3, 2)]),
    ("COROLLA", [(18, 0, 0), (2, 1, 0), (19, 2, 1), (19, 3, 2)]),
    ("240SX", [(18, 0, 0), (2, 1, 0), (20, 2, 1), (20, 3, 2)]),
    ("MIATA", [(23, 0, 0), (36, 1, 0), (37, 2, 1), (37, 3, 2)]),
    ("CIVIC", [(22, 0, 0), (4, 1, 0), (5, 2, 1), (0, 3, 2)]),
    ("PEUGOT106", [(23, 0, 0), (36, 1, 0), (37, 2, 1), (37, 3, 2)]),
    ("CORSA", [(31, 0, 0), (32, 1, 0), (36, 2, 1), (37, 3, 2)]),
    ("HUMMER", [(38, 12, 12), (13, 13, 12), (41, 14, 13), (42, 15, 14)]),
    ("NAVIGATOR", [(39, 12, 12), (13, 13, 12), (41, 14, 13), (41, 15, 14)]),
    ("ESCALADE", [(39, 12, 12), (13, 13, 12), (41, 14, 13), (41, 15, 14)]),
    ("TIBURON", [(28, 4, 6), (30, 5, 6), (7, 6, 7), (0, 7, 8)]),
    ("SENTRA", [(33, 0, 0), (34, 1, 0), (37, 2, 1), (37, 3, 2)]),
    ("CELICA", [(22, 0, 3), (2, 1, 3), (19, 2, 4), (19, 3, 5)]),
    ("IS300", [(26, 4, 6), (9, 5, 6), (27, 6, 7), (1, 7, 8)]),
    ("SUPRA", [(26, 5, 6), (9, 6, 6), (27, 7, 7), (1, 11, 8)]),
    ("GOLF", [(29, 0, 0), (21, 1, 0), (24, 2, 1), (24, 3, 2)]),
    ("A3", [(29, 0, 3), (21, 1, 3), (24, 2, 4), (24, 3, 5)]),
    ("RSX", [(4, 0, 0), (40, 1, 0), (20, 2, 1), (20, 3, 5)]),
    ("ECLIPSE", [(6, 4, 6), (7, 5, 7), (8, 6, 8), (8, 7, 8)]),
    ("TT", [(28, 5, 6), (30, 5, 6), (7, 6, 7), (0, 7, 8)]),
    ("RX8", [(10, 1, 6), (11, 1, 6), (11, 2, 7), (1, 3, 8)]),
    ("350Z", [(25, 5, 6), (26, 5, 6), (27, 6, 7), (27, 7, 8)]),
    ("G35", [(25, 4, 6), (26, 5, 6), (27, 6, 7), (27, 7, 8)]),
    ("3000GT", [(6, 4, 6), (7, 5, 7), (8, 6, 8), (8, 7, 8)]),
    ("GTO", [(14, 8, 12), (12, 9, 12), (12, 10, 13), (35, 11, 14)]),
    ("MUSTANGGT", [(14, 8, 12), (12, 9, 12), (12, 10, 13), (35, 11, 14)]),
    ("SKYLINE", [(9, 5, 6), (27, 6, 7), (1, 6, 8), (1, 7, 8)]),
    ("LANCEREVO8", [(22, 0, 3), (40, 1, 4), (5, 2, 5), (0, 3, 5)]),
    ("RX7", [(10, 1, 6), (11, 1, 7), (11, 2, 8), (1, 3, 8)]),
    ("IMPREZAWRX", [(15, 1, 6), (16, 2, 7), (17, 3, 8), (17, 3, 8)]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ea_xa_pcm_frame() {
        let mut b = vec![0xEE, 0, 1, 0, 2];
        for i in 0..28i16 {
            b.extend_from_slice(&(i * 100).to_be_bytes());
        }
        let mut out = Vec::new();
        let mut h = (0, 0);
        let end = decode_ea_xa(&b, 0, 28, &mut h, &mut out).unwrap();
        assert_eq!(end, 61);
        assert_eq!(out[5], 500);
        assert_eq!(h, (1, 2));
    }

    #[test]
    fn bit_writer_roundtrip() {
        let mut w = BitWriter::default();
        w.write(11, 0x7FF);
        w.write(5, 0b10101);
        assert_eq!(w.out, vec![0xFF, 0xF5]);
        let mut r = BitReader { b: &w.out, pos: 0 };
        assert_eq!(r.read(11).unwrap(), 0x7FF);
        assert_eq!(r.read(5).unwrap(), 0b10101);
    }

    #[test]
    fn car_table() {
        let (e, g, t) = car_sounds("350Z", 0).unwrap();
        assert_eq!(e.gin, "GIN_G35_350ZX_Comp_v2_AB.gin");
        assert_eq!(g, "GEAR_MED_Lev1.abk");
        assert_eq!(t, "TURBO_MED_0_MB.abk");
        assert_eq!(car_sounds("350z", 3).unwrap().0.gin, "GIN_350Z_Jason_shrt2.gin");
    }
}
