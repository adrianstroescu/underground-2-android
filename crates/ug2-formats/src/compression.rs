//! Decompressors for the block formats used by UG2: `JDLZ` (LZ77 variant) and
//! `HUFF` (EA canonical Huffman with run-length escapes, the "30FB" codec family).
//!
//! The HUFF decoder follows EA's reference implementation published under the
//! GPL-3.0 in the Command & Conquer Generals source release (huffdecode.cpp).

use std::borrow::Cow;

use crate::{bail, Result};

/// Every compressed block starts with a 16-byte header:
/// magic[4], u8 version, u8 flags, u16 pad, u32 unpacked size, u32 packed size.
const HEADER_LEN: usize = 16;

pub fn is_compressed(data: &[u8]) -> bool {
    matches!(data.get(0..4), Some(b"JDLZ") | Some(b"HUFF") | Some(b"RAWW"))
}

/// Decompresses `data` if it starts with a known block header, otherwise borrows it.
pub fn decompress(data: &[u8]) -> Result<Cow<'_, [u8]>> {
    match data.get(0..4) {
        Some(b"JDLZ") => Ok(Cow::Owned(jdlz(data)?)),
        Some(b"HUFF") => Ok(Cow::Owned(huff(data)?)),
        Some(b"RAWW") => {
            let len = unpacked_len(data)?;
            match data.get(HEADER_LEN..HEADER_LEN + len) {
                Some(s) => Ok(Cow::Borrowed(s)),
                None => bail!("RAWW block truncated"),
            }
        }
        _ => Ok(Cow::Borrowed(data)),
    }
}

fn unpacked_len(data: &[u8]) -> Result<usize> {
    Ok(crate::bytes::u32_at(data, 8)? as usize)
}

pub fn jdlz(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < HEADER_LEN || &data[0..4] != b"JDLZ" {
        bail!("not a JDLZ block");
    }
    let out_len = unpacked_len(data)?;
    let mut out = vec![0u8; out_len];
    let mut flags1: u32 = 1;
    let mut flags2: u32 = 1;
    let mut ip = HEADER_LEN;
    let mut op = 0usize;
    let byte = |i: usize| -> Result<u32> {
        match data.get(i) {
            Some(&b) => Ok(b as u32),
            None => bail!("JDLZ stream truncated"),
        }
    };
    while ip < data.len() && op < out_len {
        if flags1 == 1 {
            flags1 = byte(ip)? | 0x100;
            ip += 1;
        }
        if flags2 == 1 {
            flags2 = byte(ip)? | 0x100;
            ip += 1;
        }
        if flags1 & 1 != 0 {
            let (b0, b1) = (byte(ip)?, byte(ip + 1)?);
            let (len, dist) = if flags2 & 1 != 0 {
                ((b1 | ((b0 & 0xF0) << 4)) + 3, (b0 & 0x0F) + 1)
            } else {
                ((b0 & 0x1F) + 3, (b1 | ((b0 & 0xE0) << 3)) + 17)
            };
            ip += 2;
            let (len, dist) = (len as usize, dist as usize);
            if dist > op {
                bail!("JDLZ back-reference before start of output");
            }
            for i in 0..len.min(out_len - op) {
                out[op + i] = out[op + i - dist];
            }
            op += len;
            flags2 >>= 1;
        } else {
            out[op] = byte(ip)? as u8;
            op += 1;
            ip += 1;
        }
        flags1 >>= 1;
    }
    if op < out_len {
        bail!("JDLZ produced {op} of {out_len} bytes");
    }
    Ok(out)
}

/// MSB-first bit reader. Reading past the end yields zeros (bounded by `overrun`).
struct Bits<'a> {
    src: &'a [u8],
    pos: usize,
    acc: u64,
    nbits: u32,
}

impl<'a> Bits<'a> {
    fn new(src: &'a [u8]) -> Self {
        Self { src, pos: 0, acc: 0, nbits: 0 }
    }

    #[inline]
    fn refill(&mut self) {
        while self.nbits <= 56 {
            let b = self.src.get(self.pos).copied().unwrap_or(0) as u64;
            self.pos += 1;
            self.acc |= b << (56 - self.nbits);
            self.nbits += 8;
        }
    }

    /// Peek `n` (1..=32) bits.
    #[inline]
    fn peek(&mut self, n: u32) -> u32 {
        self.refill();
        (self.acc >> (64 - n)) as u32
    }

    #[inline]
    fn skip(&mut self, n: u32) {
        self.acc <<= n;
        self.nbits -= n;
    }

    #[inline]
    fn get(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let v = self.peek(n);
        self.skip(n);
        v
    }

    fn overrun(&self) -> bool {
        self.pos > self.src.len() + 16
    }

    /// EA variable-length number (SQgetnum).
    fn num(&mut self) -> Result<u32> {
        if self.peek(1) == 1 {
            return Ok(self.get(3) - 4);
        }
        let n: u32 = if self.peek(16) != 0 {
            let mut zeros = 0;
            while self.get(1) == 0 {
                zeros += 1;
            }
            2 + zeros
        } else {
            // The reference decoder counts one extra here; mirror it exactly.
            let mut n = 2;
            loop {
                n += 1;
                if self.get(1) == 1 {
                    break;
                }
                if n > 40 || self.overrun() {
                    bail!("HUFF: runaway number");
                }
            }
            n
        };
        if n > 32 {
            bail!("HUFF: number too wide ({n} bits)");
        }
        let v = if n > 16 {
            let hi = self.get(n - 16) as u64;
            let lo = self.get(16) as u64;
            lo | (hi << 16)
        } else {
            self.get(n) as u64
        };
        Ok((v + (1u64 << n) - 4) as u32)
    }
}

pub fn huff(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < HEADER_LEN || &data[0..4] != b"HUFF" {
        bail!("not a HUFF block");
    }
    let expected = unpacked_len(data)?;
    let out = huff_stream(&data[HEADER_LEN..])?;
    if out.len() != expected {
        bail!("HUFF produced {} bytes, header says {expected}", out.len());
    }
    Ok(out)
}

/// Decodes a raw EA Huffman stream (starting with the 0x30FB-family type word).
pub fn huff_stream(src: &[u8]) -> Result<Vec<u8>> {
    let mut br = Bits::new(src);
    let mut ty = br.get(16);
    if ty & 0xFF != 0xFB {
        bail!("HUFF: bad stream type {ty:#06x}");
    }
    let ulen = if ty & 0x8000 != 0 {
        if ty & 0x100 != 0 {
            br.get(16);
            br.get(16);
        }
        let hi = br.get(16);
        (hi << 16) | br.get(16)
    } else {
        if ty & 0x100 != 0 {
            br.get(8);
            br.get(16);
        }
        let hi = br.get(8);
        (hi << 16) | br.get(16)
    } as usize;
    ty &= !0x100;

    let clue = br.get(8) as u8;

    // Canonical code lengths: count of codes for each bit length.
    let mut first = [0u32; 17];
    let mut count = [0u32; 17];
    let mut base_index = [0u32; 17];
    let mut numchars = 0u32;
    let mut basecmp = 0u32;
    let mut numbits = 1usize;
    loop {
        if numbits > 16 {
            bail!("HUFF: code length table too long");
        }
        basecmp <<= 1;
        first[numbits] = basecmp;
        base_index[numbits] = numchars;
        let bitnum = br.num()?;
        count[numbits] = bitnum;
        numchars += bitnum;
        basecmp += bitnum;
        let cmp = if bitnum != 0 { (basecmp << (16 - numbits)) & 0xFFFF } else { 0 };
        numbits += 1;
        if !(bitnum == 0 || cmp != 0) {
            break;
        }
    }
    let mostbits = (numbits - 1) as u32;
    if numchars == 0 || numchars > 256 {
        bail!("HUFF: bad symbol count {numchars}");
    }

    // Symbols in code order, encoded as skips over the unused byte values.
    let mut used = [false; 256];
    let mut symbols = [0u8; 256];
    let mut next: u8 = 255;
    for sym in symbols.iter_mut().take(numchars as usize) {
        let mut leap = br.num()? as u64 + 1;
        let mut guard = 0;
        loop {
            next = next.wrapping_add(1);
            if !used[next as usize] {
                leap -= 1;
            }
            if leap == 0 {
                break;
            }
            guard += 1;
            if guard > 256 * 256 {
                bail!("HUFF: bad symbol table");
            }
        }
        used[next as usize] = true;
        *sym = next;
    }

    // Lookup table indexed by the next `mostbits` bits: (symbol, length).
    let size = 1usize << mostbits;
    let mut table = vec![(0u8, 0u8); size];
    for len in 1..=mostbits as usize {
        for k in 0..count[len] {
            let code = first[len] + k;
            let sym = symbols[(base_index[len] + k) as usize];
            let shift = mostbits as usize - len;
            let lo = (code as usize) << shift;
            let hi = ((code as usize) + 1) << shift;
            if hi > size {
                bail!("HUFF: code overflows table");
            }
            for e in &mut table[lo..hi] {
                *e = (sym, len as u8);
            }
        }
    }

    let mut out: Vec<u8> = Vec::with_capacity(ulen);
    loop {
        let (sym, len) = table[br.peek(mostbits) as usize];
        if len == 0 {
            bail!("HUFF: invalid code");
        }
        br.skip(len as u32);
        if sym != clue {
            out.push(sym);
        } else {
            let run = br.num()? as usize;
            if run != 0 {
                let Some(&last) = out.last() else { bail!("HUFF: run with no previous byte") };
                out.resize(out.len() + run, last);
            } else if br.get(1) == 1 {
                break; // end of stream
            } else {
                out.push(br.get(8) as u8);
            }
        }
        if out.len() > ulen + 0x10000 || br.overrun() {
            bail!("HUFF: stream overrun");
        }
    }

    match ty {
        0x32FB | 0xB2FB => {
            let mut acc = 0u8;
            for b in &mut out {
                acc = acc.wrapping_add(*b);
                *b = acc;
            }
        }
        0x34FB | 0xB4FB => {
            let (mut acc, mut next) = (0u8, 0u8);
            for b in &mut out {
                acc = acc.wrapping_add(*b);
                next = next.wrapping_add(acc);
                *b = next;
            }
        }
        _ => {}
    }
    Ok(out)
}
