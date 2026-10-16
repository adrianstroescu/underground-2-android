//! EA movie files (`MOVIES/*.vp6`): a stream of chunks, each a 4-byte tag and a u32 LE
//! size that includes the 8-byte header.
//!
//! * `MVhd`: u32 codec (0 = VP6), u16 width, u16 height, u32 frames, u32 largest frame,
//!   u32 rate denominator, u32 rate numerator (frames per second = den / num, e.g.
//!   1 000 000 / 33 333).
//! * `MV0K` / `MV0F`: a VP6 key / inter frame.
//! * `SCHl`, `SCCl`, `SCDl`, `SCEl`: the sound track, an EA audio stream interleaved
//!   with the video (see `audio.rs`).

use std::io::{Read, Seek, SeekFrom};

use crate::{bail, Result};

use crate::audio::{PtHeader, StreamHeader};

#[derive(Clone, Copy, Debug)]
pub struct MovieHeader {
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    /// Seconds per frame.
    pub frame_time: f64,
}

#[derive(Debug)]
pub enum MovieChunk {
    Video { key: bool, data: Vec<u8> },
    AudioHeader(StreamHeader),
    /// A whole `SCDl` block, header included.
    Audio(Vec<u8>),
    End,
}

pub struct MovieReader<R> {
    r: R,
    pos: u64,
    pub header: MovieHeader,
}

impl<R: Read + Seek> MovieReader<R> {
    pub fn open(mut r: R) -> Result<Self> {
        let mut h = [0u8; 32];
        r.seek(SeekFrom::Start(0))?;
        r.read_exact(&mut h)?;
        if &h[0..4] != b"MVhd" {
            bail!("not an EA movie");
        }
        let u16_at = |o: usize| u16::from_le_bytes([h[o], h[o + 1]]) as u32;
        let u32_at = |o: usize| u32::from_le_bytes([h[o], h[o + 1], h[o + 2], h[o + 3]]);
        let size = u32_at(4) as u64;
        let (den, num) = (u32_at(24) as f64, u32_at(28) as f64);
        let header = MovieHeader {
            width: u16_at(12),
            height: u16_at(14),
            frames: u32_at(16),
            frame_time: if den > 0.0 { num / den } else { 1.0 / 30.0 },
        };
        Ok(Self { r, pos: size, header })
    }

    pub fn next(&mut self) -> Result<MovieChunk> {
        loop {
            self.r.seek(SeekFrom::Start(self.pos))?;
            let mut h = [0u8; 8];
            match self.r.read_exact(&mut h) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(MovieChunk::End),
                Err(e) => return Err(e.into()),
            }
            let size = u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as u64;
            if size < 8 {
                return Ok(MovieChunk::End);
            }
            let mut body = vec![0u8; size as usize - 8];
            self.r.read_exact(&mut body)?;
            self.pos += size;
            match &h[0..4] {
                b"MV0K" => return Ok(MovieChunk::Video { key: true, data: body }),
                b"MV0F" => return Ok(MovieChunk::Video { key: false, data: body }),
                b"SCHl" => {
                    let mut b = h.to_vec();
                    b.extend_from_slice(&body);
                    let (pt, _) = PtHeader::parse(&b, 8)?;
                    return Ok(MovieChunk::AudioHeader(StreamHeader::from_pt(&pt)));
                }
                b"SCDl" => {
                    let mut b = h.to_vec();
                    b.extend_from_slice(&body);
                    return Ok(MovieChunk::Audio(b));
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ea_logo() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../MOVIES/ealogo.vp6");
        let Ok(f) = std::fs::File::open(path) else { return };
        let mut m = MovieReader::open(std::io::BufReader::new(f)).unwrap();
        assert_eq!((m.header.width, m.header.height, m.header.frames), (768, 512, 94));
        assert!((m.header.frame_time - 1.0 / 30.0).abs() < 1e-3);
        let (mut video, mut audio, mut sound) = (0, 0, None);
        let mut pcm = Vec::new();
        loop {
            match m.next().unwrap() {
                MovieChunk::Video { .. } => video += 1,
                MovieChunk::Audio(b) => {
                    let ch = sound.as_ref().map_or(1, |s: &StreamHeader| s.channels);
                    crate::audio::decode_ea_xa_block_be(&b, ch, &mut pcm).unwrap();
                    audio += 1
                }
                MovieChunk::AudioHeader(h) => sound = Some(h),
                MovieChunk::End => break,
            }
        }
        assert_eq!(video, 94);
        assert_eq!(audio, 94);
        let s = sound.unwrap();
        assert_eq!((s.channels, s.sample_rate), (2, 32000));
        // About 3 s of stereo sound with real signal in it.
        assert!(pcm.len() > 2 * 32000 * 2);
        assert!(pcm.iter().any(|&x| x.abs() > 1000));
    }
}
